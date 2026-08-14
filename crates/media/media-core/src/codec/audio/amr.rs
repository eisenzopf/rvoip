//! AMR-NB and AMR-WB, as media-core's pipeline sees them.
//!
//! # What this adapter is, and what it deliberately is not
//!
//! [`AudioCodec`] is a PCM-in, bytes-out interface: it has no way to say
//! "this frame is comfort noise" or "nothing was transmitted". AMR needs to
//! say both, and RFC 4867 already provides the vocabulary — the payload's own
//! table of contents carries a frame type per frame.
//!
//! So this adapter speaks **whole RFC 4867 payloads**, not bare codec frames.
//! `encode` returns a packed payload with its ToC; `decode` takes one and
//! reads the frame types back out. That is what lets DTX work through an
//! interface that cannot express it: a `NO_DATA` frame is a payload with FT 15
//! in its ToC, which is a perfectly ordinary non-empty payload, rather than an
//! empty `Vec` that the RTP layer would send as a zero-length packet.
//!
//! It also means the framing is fixed at construction. `octet-align` decides
//! the bit layout of every payload this produces, so it comes from the
//! negotiated fmtp and never from a default — guessing it produces a stream
//! the peer cannot parse at all, which is the failure mode
//! [`AmrPayloadFormat::from_negotiated`] exists to prevent.
//!
//! # Why encoding must not fail on a well-formed frame
//!
//! `audio_generation.rs` treats an encode error as fatal: it clears the
//! session's active flag and stops sending for the rest of the call. So a
//! frame of the wrong length has to be a *caller* error caught early, and
//! everything the codec can legitimately produce — speech at any rate, a SID,
//! a gap — has to come back as `Ok`.
//!
//! # Interleaving is refused at construction
//!
//! codec-core parses and carries `interleaving`, and the packer will not
//! produce an interleaved payload without a schedule to interleave against.
//! Accepting the parameter and silently sending un-interleaved frames would
//! produce a stream a conforming peer reassembles into the wrong order, so
//! this refuses the session instead.

use super::common::{AudioCodec, CodecInfo};
use crate::error::{CodecError, Error, Result};
use crate::rtp_processing::payload::amr::AmrPayloadFormat;
use crate::types::AudioFrame;
use codec_core::codecs::amr::mode::{AmrFrameType, AmrMode, AmrVariant};
use codec_core::codecs::amr::payload::{AmrPacket, AmrPayloadFrame};
use codec_core::codecs::amr::rate::CmrDamper;
use codec_core::codecs::amr::redundancy::RedundancyScheduler;
use codec_core::codecs::amr::sdp::AmrFmtp;
use codec_core::codecs::amr::AmrCodec as CoreAmrCodec;
use codec_core::types::{
    AudioCodec as CoreAudioCodec, CodecConfig as CoreCodecConfig, CodedFrame, FrameKind,
    VariableRateCodec,
};

/// One AMR stream in one direction pair.
pub struct AmrAdapter {
    /// The RFC 4867 packer, which owns the negotiated framing.
    payload: AmrPayloadFormat,
    /// The codec itself, which owns the encoder and decoder state.
    codec: CoreAmrCodec,
    /// Which variant, and therefore the frame size and clock rate.
    variant: AmrVariant,
    /// The mode being encoded at, which a peer's CMR can move.
    mode: AmrMode,
    /// The mode of the last *decoded* speech frame — what the peer is
    /// actually sending, which is not what we are encoding at.
    ///
    /// SID, NO_DATA and lost frames carry no mode of their own; the reference
    /// decoders fall back to the rate of the last frame they received. Using
    /// the *encode* mode here decoded those frames at the wrong rate whenever
    /// the two directions ran at different ones — silently, because the two
    /// modes agree in every symmetric test.
    last_decoded_mode: AmrMode,
    /// A codec mode request seen on a decoded packet, waiting to be handed to
    /// whichever object does the encoding.
    ///
    /// Not applied here. A CMR tells the *sender* what to send, and in this
    /// stack the object that decodes is a different allocation from the one
    /// that encodes ([`DialogCodecRuntime`] holds a `Mutex<StatefulCodec>`
    /// each). Applying it to the decoding object changed only that object's
    /// idea of its own transmit rate, which nothing reads — so every peer
    /// request was silently discarded.
    pending_mode_request: Option<u8>,
    /// A CMR this side wants to *emit* to the peer, asking it to change the
    /// rate it sends us. Stamped on the next outgoing payload and then
    /// cleared — one payload carries it, which is enough for the peer to see
    /// it, and repeating it every frame is what the `CmrDamper` is for.
    ///
    /// This is the emission direction, distinct from `pending_mode_request`
    /// (the peer's request to *us*). Without it every payload went out CMR=15
    /// and nothing in the stack could ever ask a peer to slow down.
    outgoing_cmr: Option<u8>,
    /// Automatic codec mode requests, when the session opted in.
    ///
    /// The damper watches which modes actually arrive and, at most once per
    /// interval, names one the peer is not using. `None` means the feature is
    /// off, which is the default: an automatic requester that damps badly
    /// oscillates the peer's rate, and never asking is better than that.
    ///
    /// This is the up-shift policy the damper implements (rtpengine's shape).
    /// A loss-driven *down*-shift — asking a peer to slow down when our
    /// receive path is losing packets — is a different policy that would need
    /// receiver statistics this object does not see, and is not implemented.
    auto_cmr: Option<CmrDamper>,
    /// A request the damper just produced, waiting to be routed to the
    /// encoding object. Same seam, and same reason, as
    /// [`Self::pending_mode_request`].
    automatic_cmr: Option<u8>,
    /// The peer's declared `max-red` ceiling, kept so a caller can ask for a
    /// depth and be refused if the peer never permitted it.
    negotiated_max_red: Option<u16>,
    /// Outgoing redundancy, when a caller asked for it and the peer allows it.
    ///
    /// `None` is the ordinary case and means one frame per payload. We
    /// advertise `max-red=0`, so a peer has no reason to send us repeats
    /// either; the receive side handles multi-frame payloads regardless,
    /// because a peer may bundle for its own reasons.
    redundancy: Option<RedundancyScheduler>,
}

impl AmrAdapter {
    /// Build an adapter from the negotiated codec name and fmtp.
    ///
    /// `codec_name` is `"AMR"` or `"AMR-WB"`; `fmtp` is the negotiated
    /// attribute, `None` meaning the peer sent none and the defaults apply.
    ///
    /// # Errors
    ///
    /// When the name is not an AMR one, when the fmtp is malformed, when it
    /// requests interleaving, or when the codec cannot be constructed for the
    /// negotiated mode set.
    pub fn new(payload_type: u8, codec_name: &str, fmtp: Option<&str>) -> Result<Self> {
        let payload =
            AmrPayloadFormat::from_negotiated(payload_type, codec_name, fmtp).map_err(|error| {
                Error::Codec(CodecError::InvalidParameters {
                    details: format!("AMR fmtp: {error}"),
                })
            })?;

        // Refused on the *send* side, which is what the parameter obliges.
        // RFC 4867 §8.1 makes fmtp declarative: a peer naming `interleaving`
        // is asking to receive interleaved payloads, so honouring it means
        // spreading our frame-blocks across packets — which we do not do.
        // Receive-side reassembly does exist
        // (`codec_core::codecs::amr::interleave::Deinterleaver`), so a peer
        // that interleaves *toward* us could be handled; it is this direction
        // that is missing, and accepting the session would send frames a
        // conforming peer reassembles out of order.
        if payload.codec().config().interleaving {
            return Err(Error::Codec(CodecError::InvalidParameters {
                details: "AMR interleaving is negotiated, but this endpoint does not \
                          interleave on transmit; refusing the session rather than sending \
                          frames a conforming peer would reassemble out of order"
                    .to_string(),
            })
            .into());
        }

        let variant = payload.variant();
        let mode = payload.mode();

        // The whole negotiated set, not just the starting mode. Collapsing it
        // to `1 << mode` makes every codec mode request unsatisfiable, so the
        // stream is pinned at whatever rate it opened on -- which looks
        // correct until a peer with a congested downlink asks for a lower one
        // and is ignored.
        let parsed = AmrFmtp::parse(variant, fmtp.unwrap_or("")).map_err(|error| {
            Error::Codec(CodecError::InvalidParameters {
                details: format!("AMR fmtp: {error}"),
            })
        })?;
        let mut mode_set = 0u16;
        for permitted in parsed.active_modes().modes() {
            mode_set |= 1u16 << permitted.index();
        }

        let mut config = match variant {
            AmrVariant::NarrowBand => CoreCodecConfig::amr_nb(),
            AmrVariant::WideBand => CoreCodecConfig::amr_wb(),
        };
        config.parameters.amr.mode_set = mode_set;
        // The rate-change constraints, which only bite once codec mode
        // requests actually move the encoder. Setting only `mode_set` left
        // codec-core on its defaults (period 1, neighbour off), so a peer that
        // negotiated `mode-change-period=2` would get a change on every frame
        // and one that negotiated `mode-change-neighbor=1` would get arbitrary
        // jumps -- both of which it explicitly asked us not to do.
        config.parameters.amr.mode_change_period = parsed.mode_change_period;
        config.parameters.amr.mode_change_neighbor = parsed.mode_change_neighbor;

        let codec = CoreAmrCodec::new(&config).map_err(|error| {
            Error::Codec(CodecError::InvalidParameters {
                details: format!("AMR codec: {error}"),
            })
        })?;

        Ok(Self {
            payload,
            codec,
            variant,
            mode,
            last_decoded_mode: mode,
            pending_mode_request: None,
            outgoing_cmr: None,
            auto_cmr: None,
            automatic_cmr: None,
            negotiated_max_red: parsed.max_red,
            redundancy: None,
        })
    }

    /// Let this session ask its peer to change rate on its own.
    ///
    /// Off unless a deployment asks for it — see
    /// [`AMR_AUTO_CMR_PARAMETER`](crate::relay::controller::AMR_AUTO_CMR_PARAMETER).
    /// The damper observes the modes the peer actually sends and, once per
    /// interval, requests at most one step toward a mode it is not using;
    /// the request rides out on the next payload through the same field an
    /// explicit `request_peer_mode` uses.
    ///
    /// `interval_frames` is counted in 20 ms frame-blocks, so 250 is five
    /// seconds — the interval rtpengine documents.
    ///
    /// # Errors
    ///
    /// When `interval_frames` is zero, which would request on every frame.
    pub fn set_auto_cmr(&mut self, enabled: bool, interval_frames: u32) -> Result<()> {
        if !enabled {
            self.auto_cmr = None;
            return Ok(());
        }
        let mode_set = self.codec.mode_set().clone();
        let damper = CmrDamper::new(mode_set, interval_frames).map_err(|error| {
            Error::Codec(CodecError::InvalidParameters {
                details: format!("AMR automatic CMR: {error}"),
            })
        })?;
        self.auto_cmr = Some(damper);
        Ok(())
    }

    /// Repeat recent frames in each outgoing payload, up to `depth` frames
    /// per payload (1 disables it).
    ///
    /// Bounded by the peer's negotiated `max-red`: it declares how long a
    /// frame may keep being retransmitted, and asking for more than it allows
    /// is refused rather than quietly clamped — a caller that believes it has
    /// three-deep protection and silently gets one is worse off than one told
    /// no.
    ///
    /// # Errors
    ///
    /// When `depth` exceeds what the peer's `max-red` permits, or exceeds the
    /// 32 frame-blocks a payload can address.
    pub fn set_redundancy_depth(&mut self, depth: usize) -> Result<()> {
        if depth <= 1 {
            self.redundancy = None;
            return Ok(());
        }
        let scheduler =
            RedundancyScheduler::new(self.negotiated_max_red, depth).map_err(|error| {
                Error::Codec(CodecError::InvalidParameters {
                    details: format!("AMR redundancy: {error}"),
                })
            })?;
        self.redundancy = Some(scheduler);
        Ok(())
    }

    /// Frames per outgoing payload: 1 unless redundancy is on.
    #[must_use]
    pub fn redundancy_depth(&self) -> usize {
        self.redundancy
            .as_ref()
            .map_or(1, RedundancyScheduler::depth)
    }

    /// Stage a CMR unless one is already waiting, so an automatic request
    /// never displaces an explicit one.
    pub const fn request_peer_mode_if_idle(&mut self, mode_index: u8) {
        if self.outgoing_cmr.is_none() {
            self.outgoing_cmr = Some(mode_index);
        }
    }

    /// Take the automatic codec mode request the damper produced, if any.
    ///
    /// Mirrors [`Self::take_mode_request`]: the caller owns routing it to
    /// whichever object does the encoding.
    pub const fn take_automatic_cmr(&mut self) -> Option<u8> {
        self.automatic_cmr.take()
    }

    /// Allow this session's encoder to replace silence with comfort noise.
    ///
    /// Off unless a deployment asks for it — see
    /// [`AMR_DTX_PARAMETER`](crate::relay::controller::AMR_DTX_PARAMETER). It
    /// changes what goes on the wire, and the receive side handles a peer's
    /// SID and `NO_DATA` frames whether or not this is set, so nothing about
    /// interoperating depends on turning it on.
    pub fn set_allow_dtx(&mut self, allow: bool) {
        self.codec.set_allow_dtx(allow);
    }

    /// Samples in one 20 ms frame at this variant's rate: 160 or 320.
    #[must_use]
    pub const fn frame_samples(&self) -> usize {
        self.variant.frame_samples()
    }

    /// The variant's clock rate in hertz: 8000 or 16000.
    ///
    /// AMR-WB's RTP clock is 16 kHz, and everything that derives a packet's
    /// sample count from a hard-coded 8000 gets AMR-WB wrong by a factor of
    /// two.
    #[must_use]
    pub const fn clock_rate(&self) -> u32 {
        match self.variant {
            AmrVariant::NarrowBand => 8_000,
            AmrVariant::WideBand => 16_000,
        }
    }

    /// Take any codec mode request this adapter has decoded since the last
    /// call, so the caller can hand it to the encoding side.
    ///
    /// Returns `None` on the common path — a packet whose CMR is 15, "no
    /// request", or no packet at all.
    pub const fn take_mode_request(&mut self) -> Option<u8> {
        self.pending_mode_request.take()
    }

    /// Ask the peer to send us a different mode: the next payload this adapter
    /// packs carries the CMR, once.
    ///
    /// Silently ignored when the mode is outside the negotiated set — a CMR is
    /// a request the peer may decline, so asking for an impossible one is a
    /// no-op rather than an error. This is the emission counterpart of
    /// [`apply_mode_request`](Self::apply_mode_request), which handles a CMR
    /// coming the other way.
    pub fn request_peer_mode(&mut self, mode_index: u8) {
        if AmrMode::new(self.variant, mode_index).is_ok() {
            self.outgoing_cmr = Some(mode_index);
        }
    }

    /// The mode of the last speech frame decoded from the peer — what the peer
    /// is actually sending right now. Lets a live test confirm a requested
    /// mode change was honoured on the wire rather than only sent.
    pub const fn last_decoded_mode(&self) -> u8 {
        self.last_decoded_mode.index()
    }

    /// Honour a peer's codec mode request.
    ///
    /// Silently ignored when the requested mode is outside the negotiated
    /// mode set: a CMR is a *request*, and RFC 4867 §3.4.1 leaves an
    /// unsatisfiable one to the encoder's discretion. Refusing the packet
    /// instead would drop audio over a field the sender may set freely.
    ///
    /// Also ignored when the negotiated `mode-change-period` says this frame
    /// is not a permitted change point, or when `mode-change-neighbor` allows
    /// only a single step — [`codec_core`]'s `ModeChangePolicy` decides, and
    /// the mode actually in effect is read back rather than assumed.
    pub fn apply_mode_request(&mut self, cmr: Option<u8>) {
        let Some(index) = cmr else { return };
        let Ok(requested) = AmrMode::new(self.variant, index) else {
            return;
        };
        if self.codec.set_mode(requested.index()).is_ok() {
            // Read back rather than assume: the change policy may have
            // deferred this request to a later frame-block, or moved one step
            // toward it instead of all the way.
            if let Ok(effective) = AmrMode::new(self.variant, self.codec.current_mode()) {
                self.mode = effective;
                let _ = self.payload.set_mode(effective);
            }
        }
    }
}

impl AudioCodec for AmrAdapter {
    /// Encode exactly one frame into an RFC 4867 payload.
    ///
    /// # Errors
    ///
    /// Only when `audio_frame` is not exactly one frame's worth of samples.
    /// Everything the codec itself can produce — speech, a SID, a gap — is a
    /// payload, because the frame type rides in the ToC.
    fn encode(&mut self, audio_frame: &AudioFrame) -> Result<Vec<u8>> {
        let wanted = self.frame_samples();
        if audio_frame.samples.len() != wanted {
            return Err(CodecError::InvalidFrameSize {
                expected: wanted,
                actual: audio_frame.samples.len(),
            }
            .into());
        }

        let coded = self
            .codec
            .encode_frame(&audio_frame.samples)
            .map_err(|error| {
                Error::Codec(CodecError::InvalidParameters {
                    details: format!("AMR encode: {error}"),
                })
            })?;

        let frame_type = match coded.kind {
            FrameKind::Speech => {
                AmrFrameType::Speech(AmrMode::new(self.variant, coded.mode).unwrap_or(self.mode))
            }
            FrameKind::ComfortNoise => AmrFrameType::Sid(self.variant),
            FrameKind::NoData => AmrFrameType::NoData,
            FrameKind::Lost => AmrFrameType::SpeechLost,
        };
        let frame = AmrPayloadFrame::new(frame_type, true, coded.data).map_err(|error| {
            Error::Codec(CodecError::InvalidParameters {
                details: format!("AMR frame: {error}"),
            })
        })?;

        // With redundancy on, the payload carries recent frames as well as
        // this one, oldest first. The caller stamps the RTP timestamp, and
        // RFC 4867 §4.3 requires it to name the *oldest* frame —
        // `RedundancyScheduler::payload_timestamp` computes that.
        let packet = match self.redundancy.as_mut() {
            Some(scheduler) => AmrPacket {
                cmr: None,
                interleaving: None,
                frames: scheduler.next_payload(frame),
            },
            None => AmrPacket::single(frame),
        }
        .with_cmr(self.outgoing_cmr.take());
        self.payload.codec().pack(&packet).map_err(|error| {
            Error::Codec(CodecError::InvalidParameters {
                details: format!("AMR pack: {error}"),
            })
        })
    }

    /// Decode one RFC 4867 payload.
    ///
    /// A payload may carry several frames; all of them are decoded and
    /// concatenated, so the returned frame is a whole multiple of
    /// [`frame_samples`](Self::frame_samples).
    ///
    /// # Errors
    ///
    /// When the payload does not parse under the negotiated framing, or when a
    /// frame's contents are not decodable.
    fn decode(&mut self, encoded_data: &[u8]) -> Result<AudioFrame> {
        let packet = self.payload.codec().unpack(encoded_data).map_err(|error| {
            Error::Codec(CodecError::InvalidParameters {
                details: format!("AMR payload: {error}"),
            })
        })?;

        // The peer's mode request rides on the packet it arrived with and
        // applies to what *this* end sends next — which is a different object
        // from this one. Record it; the caller routes it.
        if packet.cmr.is_some() {
            self.pending_mode_request = packet.cmr;
        }

        let mut samples = Vec::with_capacity(packet.frames.len() * self.frame_samples());
        for frame in &packet.frames {
            // Every arriving frame-block advances the damper's interval, and
            // speech frames also tell it which mode the peer chose. SID and
            // NO_DATA count toward the interval but carry no mode, so a peer
            // that goes quiet cannot make us request a change on no evidence.
            if let Some(damper) = self.auto_cmr.as_mut() {
                if let AmrFrameType::Speech(mode) = frame.frame_type {
                    damper.observe(mode);
                }
                if let Some(request) = damper.advance() {
                    // Recorded, not applied. This object decodes; the object
                    // that stamps CMRs on outgoing payloads is a different
                    // allocation behind a different lock, exactly as for the
                    // peer's own request above. `DialogCodecRuntime::decode`
                    // hands it across.
                    self.automatic_cmr = Some(request.index());
                }
            }
            let coded = match frame.frame_type {
                AmrFrameType::Speech(mode) => {
                    self.last_decoded_mode = mode;
                    CodedFrame {
                        kind: FrameKind::Speech,
                        mode: mode.index(),
                        data: frame.data.clone(),
                        quality_ok: frame.quality_ok,
                    }
                }
                // SID, NO_DATA and lost frames have no mode of their own.
                // The reference decoders continue at the rate of the last
                // frame the *peer* sent — not at whatever rate we happen to
                // be encoding, which is a different direction entirely.
                AmrFrameType::Sid(_) => CodedFrame {
                    kind: FrameKind::ComfortNoise,
                    mode: self.last_decoded_mode.index(),
                    data: frame.data.clone(),
                    quality_ok: frame.quality_ok,
                },
                AmrFrameType::NoData => CodedFrame {
                    kind: FrameKind::NoData,
                    mode: self.last_decoded_mode.index(),
                    data: Vec::new(),
                    quality_ok: true,
                },
                AmrFrameType::SpeechLost => CodedFrame {
                    kind: FrameKind::Lost,
                    mode: self.last_decoded_mode.index(),
                    data: Vec::new(),
                    quality_ok: false,
                },
            };
            let pcm = self.codec.decode_frame(&coded).map_err(|error| {
                Error::Codec(CodecError::InvalidParameters {
                    details: format!("AMR decode: {error}"),
                })
            })?;
            // 3GPP defines AMR-NB's output as 13-bit and AMR-WB's as 14-bit,
            // and both references mask before handing samples to the
            // application -- but in different places: narrowband inside
            // `Speech_Decode_Frame` (the library), wideband in `decoder.c`
            // (the driver). codec-core mirrors each faithfully, so its
            // narrowband output arrives masked and its wideband output does
            // not.
            //
            // This adapter is the application boundary, so it is where the
            // driver's mask belongs. Without it the two variants of one trait
            // hand callers different precision: measured, ~6000 of 8000
            // wideband samples per rate carried nonzero low bits.
            let mask = match self.variant {
                AmrVariant::NarrowBand => !7i16,
                AmrVariant::WideBand => !3i16,
            };
            samples.extend(pcm.iter().map(|&s| s & mask));
        }

        Ok(AudioFrame::new(samples, self.clock_rate(), 1, 0))
    }

    fn get_info(&self) -> CodecInfo {
        CodecInfo {
            name: self.variant.sdp_name().to_string(),
            sample_rate: self.clock_rate(),
            channels: 1,
            bitrate: self.mode.bitrate(),
        }
    }

    fn reset(&mut self) {
        let _ = self.codec.reset();
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pcm(samples: usize, rate: u32) -> AudioFrame {
        // A tone rather than silence: an all-zero frame encodes to the same
        // bits at every rate, so a mode mix-up would not show.
        let data: Vec<i16> = (0..samples)
            .map(|i| {
                let phase = (i as f32) * 0.07;
                (phase.sin() * 6000.0) as i16
            })
            .collect();
        AudioFrame::new(data, rate, 1, 0)
    }

    /// Both variants hand the caller the precision 3GPP defines.
    ///
    /// AMR-NB is 13-bit and AMR-WB is 14-bit. The two references mask in
    /// different places — narrowband in the library, wideband in the driver —
    /// and codec-core mirrors each, so the masking has to happen here, at the
    /// application boundary, or the same trait yields different precision
    /// depending on the variant.
    #[test]
    fn decoded_samples_carry_the_precision_the_spec_defines() {
        let mut checked = 0;
        #[cfg(feature = "amr-nb")]
        {
            let mut codec = AmrAdapter::new(96, "AMR", None).expect("constructs");
            let payload = codec.encode(&pcm(160, 8_000)).expect("encodes");
            let out = codec.decode(&payload).expect("decodes");
            assert!(
                out.samples.iter().all(|s| s & 7 == 0),
                "AMR-NB output must be 13-bit"
            );
            assert!(out.samples.iter().any(|&s| s != 0), "vacuous on silence");
            checked += 1;
        }
        #[cfg(feature = "amr-wb")]
        {
            let mut codec = AmrAdapter::new(97, "AMR-WB", None).expect("constructs");
            let payload = codec.encode(&pcm(320, 16_000)).expect("encodes");
            let out = codec.decode(&payload).expect("decodes");
            assert!(
                out.samples.iter().all(|s| s & 3 == 0),
                "AMR-WB output must be 14-bit"
            );
            assert!(out.samples.iter().any(|&s| s != 0), "vacuous on silence");
            checked += 1;
        }
        assert!(checked > 0, "no variant was compiled in");
    }

    /// Every compiled-in variant round-trips, and the framing survives.
    ///
    /// Gated per variant rather than assuming both: a build with only
    /// `amr-nb` refuses to construct a wideband codec, and a test that
    /// hard-codes both fails there for a reason that has nothing to do with
    /// the adapter.
    #[test]
    fn a_frame_round_trips_at_every_compiled_variant() {
        let mut variants: Vec<(&str, usize, u32)> = Vec::new();
        #[cfg(feature = "amr-nb")]
        variants.push(("AMR", 160, 8_000));
        #[cfg(feature = "amr-wb")]
        variants.push(("AMR-WB", 320, 16_000));
        assert!(
            !variants.is_empty(),
            "this module needs at least one variant"
        );

        for (name, samples, rate) in variants {
            let mut codec = AmrAdapter::new(96, name, None).expect("constructs");
            assert_eq!(codec.frame_samples(), samples, "{name} frame size");
            assert_eq!(codec.clock_rate(), rate, "{name} clock rate");

            let payload = codec.encode(&pcm(samples, rate)).expect("encodes");
            assert!(
                !payload.is_empty(),
                "{name}: an encoded frame is never empty"
            );

            let decoded = codec.decode(&payload).expect("decodes");
            assert_eq!(decoded.samples.len(), samples, "{name} decoded length");
            assert_eq!(decoded.sample_rate, rate, "{name} decoded rate");
            assert!(
                decoded.samples.iter().any(|&s| s != 0),
                "{name}: the round trip produced silence"
            );
        }
    }

    /// Frames that carry no mode of their own decode at the rate of the last
    /// frame the *peer* sent, never at the rate we happen to encode at.
    ///
    /// SID, NO_DATA and lost frames used to borrow `self.mode` — the encode
    /// mode. The two agree in every symmetric test, so the defect only shows
    /// when the directions run at different rates: here the peer speaks at
    /// 6.60 kbit/s while a CMR moves *our* encoder, and concealment of a lost
    /// frame must not move with it.
    #[test]
    #[cfg(feature = "amr-wb")]
    fn no_mode_frames_decode_at_the_peers_last_speech_mode_not_ours() {
        use codec_core::codecs::amr::payload::{AmrPayloadCodec, AmrPayloadConfig};

        // A payload whose only frame is FT 14 — "speech lost", wideband's
        // in-band loss marker — in the same bandwidth-efficient framing the
        // fmtp-less adapters below negotiate.
        let lost_payload = {
            let packer = AmrPayloadCodec::new(AmrPayloadConfig {
                variant: AmrVariant::WideBand,
                octet_aligned: false,
                crc: false,
                robust_sorting: false,
                interleaving: false,
            })
            .expect("packer");
            let frame = AmrPayloadFrame::new(AmrFrameType::SpeechLost, true, Vec::new())
                .expect("a lost frame carries no data");
            packer.pack(&AmrPacket::single(frame)).expect("packs")
        };

        // The peer: an encoder pinned to the lowest rate by its mode-set.
        let low_payloads: Vec<Vec<u8>> = {
            let mut encoder =
                AmrAdapter::new(97, "AMR-WB", Some("mode-set=0")).expect("constructs");
            (0..3)
                .map(|_| encoder.encode(&pcm(320, 16_000)).expect("encodes"))
                .collect()
        };
        // A different peer speaking at the highest rate, for the
        // non-vacuity half below.
        let high_payloads: Vec<Vec<u8>> = {
            let mut encoder = AmrAdapter::new(97, "AMR-WB", None).expect("constructs");
            (0..3)
                .map(|_| encoder.encode(&pcm(320, 16_000)).expect("encodes"))
                .collect()
        };

        let conceal_after = |speech: &[Vec<u8>], cmr: Option<u8>| -> Vec<i16> {
            let mut decoder = AmrAdapter::new(97, "AMR-WB", None).expect("constructs");
            for payload in speech {
                decoder.decode(payload).expect("decodes");
            }
            // Moving the *encode* mode between the speech and the loss is the
            // mutation this test exists to catch.
            decoder.apply_mode_request(cmr);
            decoder.decode(&lost_payload).expect("conceals").samples
        };

        let concealed = conceal_after(&low_payloads, None);
        let concealed_with_encoder_moved = conceal_after(&low_payloads, Some(0));
        assert_eq!(
            concealed, concealed_with_encoder_moved,
            "concealment followed the encode mode: a CMR that moves what we \
             send must not change how we conceal what the peer lost"
        );

        // Non-vacuity: concealment genuinely depends on the peer's rate, so
        // the equality above cannot hold by concealment ignoring mode.
        let concealed_after_high = conceal_after(&high_payloads, None);
        assert_ne!(
            concealed, concealed_after_high,
            "concealment after 6.60 kbit/s speech and after 23.85 kbit/s \
             speech produced identical samples — the comparison proves nothing"
        );
    }

    /// The full CMR round trip: a request emitted on one side is packed onto
    /// the wire, read back by the peer's decoder, and moves that peer's
    /// encode mode — which the requester then sees on the frames coming back.
    ///
    /// This is the emission direction that did not exist: every payload used
    /// to go out CMR=15. Mutation-guarded — asserting the request was *not*
    /// visible before it was made, and that a request for the current mode is
    /// a no-op, so a stubbed emitter fails.
    #[test]
    #[cfg(feature = "amr-wb")]
    fn a_requested_mode_change_crosses_the_wire_and_moves_the_peer() {
        use codec_core::codecs::amr::mode::AmrFrameType;

        // A is the requester; B is the peer whose rate A wants to change.
        // Both negotiate the full mode set so any request is satisfiable.
        let mut side_a = AmrAdapter::new(97, "AMR-WB", None).expect("constructs A");
        let mut side_b = AmrAdapter::new(97, "AMR-WB", None).expect("constructs B");

        // A's payload before any request carries no CMR (15 = none).
        let quiet = side_a.encode(&pcm(320, 16_000)).expect("A encodes");
        let quiet_cmr = side_a.payload.codec().unpack(&quiet).expect("unpack").cmr;
        assert_eq!(
            quiet_cmr, None,
            "no request yet, so CMR must be none (15 unpacks to None)"
        );

        // A asks B to drop to mode 0 (6.60 kbit/s), the lowest.
        side_a.request_peer_mode(0);
        let request_payload = side_a
            .encode(&pcm(320, 16_000))
            .expect("A encodes with CMR");
        let request_cmr = side_a
            .payload
            .codec()
            .unpack(&request_payload)
            .expect("unpack")
            .cmr;
        assert_eq!(request_cmr, Some(0), "the emitted payload must carry CMR 0");

        // And only one payload carries it — the field clears after emission.
        let next = side_a.encode(&pcm(320, 16_000)).expect("A encodes again");
        assert_eq!(
            side_a.payload.codec().unpack(&next).expect("unpack").cmr,
            None,
            "the CMR must not repeat every frame"
        );

        // B decodes A's request-bearing payload and hands the CMR to its own
        // encoder, exactly as DialogCodecRuntime does across the two locks.
        side_b.decode(&request_payload).expect("B decodes");
        if let Some(cmr) = side_b.take_mode_request() {
            side_b.apply_mode_request(Some(cmr));
        } else {
            panic!("B did not see A's CMR");
        }

        // B now encodes at the requested mode; A decodes it and sees the move.
        let b_payload = side_b
            .encode(&pcm(320, 16_000))
            .expect("B encodes at new rate");
        let b_frame = side_b
            .payload
            .codec()
            .unpack(&b_payload)
            .expect("unpack")
            .frames
            .remove(0);
        assert_eq!(
            b_frame.frame_type,
            AmrFrameType::Speech(AmrMode::new(AmrVariant::WideBand, 0).unwrap()),
            "B's encoder should have moved to mode 0"
        );
        side_a.decode(&b_payload).expect("A decodes B");
        assert_eq!(
            side_a.last_decoded_mode(),
            0,
            "A must observe the peer now sending mode 0"
        );
    }

    /// A request for a mode outside the negotiated set is declined, not
    /// packed as a malformed nibble.
    #[test]
    #[cfg(feature = "amr-nb")]
    fn an_out_of_range_mode_request_is_ignored() {
        let mut codec = AmrAdapter::new(96, "AMR", None).expect("constructs");
        codec.request_peer_mode(9); // AMR-NB has modes 0..=7
        let payload = codec.encode(&pcm(160, 8_000)).expect("encodes");
        assert_eq!(
            codec.payload.codec().unpack(&payload).expect("unpack").cmr,
            None,
            "an unsatisfiable request must not reach the wire"
        );
    }

    /// A frame of the wrong length is refused rather than padded or split.
    ///
    /// This is the one error `encode` may return, and it has to be returned
    /// rather than worked around: `audio_generation` treats an encode failure
    /// as fatal for the session, so a codec that silently accepted a short
    /// frame would drift against the far end instead of failing loudly here.
    #[test]
    #[cfg(feature = "amr-nb")]
    fn a_misframed_buffer_is_refused() {
        let mut codec = AmrAdapter::new(96, "AMR", None).expect("constructs");
        for length in [0usize, 80, 159, 161, 320] {
            assert!(
                codec.encode(&pcm(length, 8_000)).is_err(),
                "{length} samples should not encode as an AMR-NB frame"
            );
        }
        // And the right length still works afterwards -- the refusal must not
        // leave the codec unusable.
        assert!(codec.encode(&pcm(160, 8_000)).is_ok());
    }

    /// The negotiated framing reaches the wire, and the two framings differ.
    ///
    /// If `octet-align` were ignored the two payloads would be identical and
    /// this would pass having proved nothing, so the difference is asserted
    /// before the length is.
    #[test]
    #[cfg(feature = "amr-nb")]
    fn octet_alignment_changes_the_payload() {
        let frame = pcm(160, 8_000);
        let mut aligned = AmrAdapter::new(96, "AMR", Some("octet-align=1")).expect("constructs");
        let mut efficient = AmrAdapter::new(96, "AMR", Some("octet-align=0")).expect("constructs");

        let a = aligned.encode(&frame).expect("encodes");
        let b = efficient.encode(&frame).expect("encodes");
        assert_ne!(a, b, "the two framings produced identical payloads");
        assert!(a.len() > b.len(), "octet-aligned framing is the longer one");

        // Each decodes its own and, being a different framing, generally not
        // the other's -- but the interesting assertion is that its own works.
        assert!(aligned.decode(&a).is_ok());
        assert!(efficient.decode(&b).is_ok());
    }

    /// Interleaving is refused at construction rather than silently dropped.
    #[test]
    #[cfg(feature = "amr-nb")]
    fn negotiated_interleaving_is_refused() {
        let Err(err) = AmrAdapter::new(96, "AMR", Some("octet-align=1;interleaving=2")) else {
            panic!("interleaving must be refused");
        };
        assert!(err.to_string().contains("interleaving"), "{err}");
    }

    /// A name that is not AMR does not construct one.
    #[test]
    fn only_amr_names_construct() {
        for name in ["PCMU", "opus", "G729", "AMR-NB", ""] {
            assert!(
                AmrAdapter::new(96, name, None).is_err(),
                "`{name}` should not construct an AMR adapter"
            );
        }
    }

    /// The negotiated rate-change constraints reach the codec.
    ///
    /// Only `mode_set` was passed through, so `mode-change-period=2` and
    /// `mode-change-neighbor=1` were parsed out of the peer's fmtp and then
    /// dropped on the floor. Latent while codec mode requests went nowhere;
    /// live the moment they started working.
    ///
    /// The period counts *frame-blocks*, so each request below is separated by
    /// an encoded frame. Two requests inside one frame-block only ever take
    /// the first, at any period — which is correct, and would make a test
    /// without the intervening frame pass for the wrong reason.
    #[test]
    #[cfg(feature = "amr-nb")]
    fn the_negotiated_rate_change_constraints_reach_the_codec() {
        let frame = pcm(160, 8_000);

        let mut deferred = AmrAdapter::new(96, "AMR", Some("mode-set=0,4,7; mode-change-period=2"))
            .expect("constructs");
        let mut prompt = AmrAdapter::new(96, "AMR", Some("mode-set=0,4,7")).expect("constructs");

        for codec in [&mut deferred, &mut prompt] {
            assert_ne!(
                codec.mode.index(),
                4,
                "vacuous if it starts at the first target"
            );
            codec.apply_mode_request(Some(4));
            assert_eq!(codec.mode.index(), 4, "the first request should land");
            codec.encode(&frame).expect("encodes");
            codec.apply_mode_request(Some(0));
        }

        assert_eq!(
            prompt.mode.index(),
            0,
            "period defaults to 1, so a request one frame-block later lands"
        );
        assert_eq!(
            deferred.mode.index(),
            4,
            "mode-change-period=2 should still be deferring after one frame-block"
        );

        // And it is a deferral rather than a refusal: one more frame-block and
        // the same request takes.
        deferred.encode(&frame).expect("encodes");
        deferred.apply_mode_request(Some(0));
        assert_eq!(
            deferred.mode.index(),
            0,
            "the second frame-block should allow it"
        );
    }

    /// A peer's mode request moves the encoder, and an impossible one does not
    /// drop the packet.    /// A peer's mode request moves the encoder, and an impossible one does not
    /// drop the packet.
    #[test]
    #[cfg(feature = "amr-nb")]
    fn a_mode_request_is_honoured_or_ignored_but_never_fatal() {
        let mut codec = AmrAdapter::new(96, "AMR", Some("mode-set=0,7")).expect("constructs");
        let started = codec.mode.index();

        codec.apply_mode_request(Some(0));
        assert_eq!(codec.mode.index(), 0, "a permitted request should be taken");
        assert_ne!(started, 0, "the test is vacuous if it started there");

        // Outside the mode set, and outside the variant entirely.
        codec.apply_mode_request(Some(3));
        codec.apply_mode_request(Some(9));
        codec.apply_mode_request(None);
        assert_eq!(
            codec.mode.index(),
            0,
            "an unsatisfiable request must be ignored"
        );

        // And the codec still works.
        assert!(codec.encode(&pcm(160, 8_000)).is_ok());
    }

    /// The reported info matches what the pipeline needs to size packets.
    #[test]
    #[cfg(all(feature = "amr-nb", feature = "amr-wb"))]
    fn the_reported_info_matches_the_variant() {
        let nb = AmrAdapter::new(96, "AMR", None).expect("constructs");
        let wb = AmrAdapter::new(97, "AMR-WB", None).expect("constructs");
        assert_eq!(nb.get_info().sample_rate, 8_000);
        assert_eq!(wb.get_info().sample_rate, 16_000);
        assert_eq!(nb.get_info().name, "AMR");
        assert_eq!(wb.get_info().name, "AMR-WB");
        assert_eq!(nb.get_info().channels, 1);
        assert!(nb.get_info().bitrate >= 4_750 && nb.get_info().bitrate <= 12_200);
        assert!(wb.get_info().bitrate >= 6_600 && wb.get_info().bitrate <= 23_850);
    }
    /// Redundancy is refused unless the peer's `max-red` permits it, and does
    /// what it says when it is allowed.
    ///
    /// The refusal half matters most: `max-red` is the peer telling us how
    /// long we may keep retransmitting a frame, and quietly clamping a
    /// too-deep request would leave a caller believing it had protection it
    /// does not have.
    #[test]
    fn redundancy_depth_is_bounded_by_the_peers_max_red() {
        // No max-red declared: no redundancy permitted beyond a single copy.
        let mut plain = AmrAdapter::new(96, "AMR", Some("octet-align=1")).expect("constructs");
        assert_eq!(plain.redundancy_depth(), 1);
        assert!(
            plain.set_redundancy_depth(2).is_err(),
            "a peer that declared no max-red has not permitted redundancy"
        );
        assert_eq!(
            plain.redundancy_depth(),
            1,
            "a refused request must change nothing"
        );

        // max-red=40 permits three transmissions of a frame (0, 20, 40 ms).
        let mut permitted =
            AmrAdapter::new(96, "AMR", Some("octet-align=1; max-red=40")).expect("constructs");
        permitted
            .set_redundancy_depth(3)
            .expect("40ms allows depth 3");
        assert_eq!(permitted.redundancy_depth(), 3);
        assert!(
            permitted.set_redundancy_depth(4).is_err(),
            "40ms does not allow a fourth transmission"
        );
        assert_eq!(permitted.redundancy_depth(), 3);

        // Depth 1 turns it off again.
        permitted.set_redundancy_depth(1).expect("disables");
        assert_eq!(permitted.redundancy_depth(), 1);
    }

    /// With redundancy on, payloads grow to carry the repeats — and the
    /// stream still decodes.
    #[test]
    fn redundant_payloads_carry_previous_frames_and_still_decode() {
        let mut sender =
            AmrAdapter::new(96, "AMR", Some("octet-align=1; max-red=20")).expect("constructs");
        sender.set_redundancy_depth(2).expect("20ms allows depth 2");
        let mut receiver =
            AmrAdapter::new(96, "AMR", Some("octet-align=1; max-red=20")).expect("constructs");

        let pcm: Vec<i16> = (0..160)
            .map(|i| ((f64::from(i) * 0.09).sin() * 6_000.0) as i16)
            .collect();
        let frame = AudioFrame::new(pcm, 8_000, 1, 0);

        let first = sender.encode(&frame).expect("first payload");
        let second = sender.encode(&frame).expect("second payload");
        assert!(
            second.len() > first.len(),
            "depth 2 must bundle the previous frame: {} then {}",
            first.len(),
            second.len()
        );

        // The receive side takes a multi-frame payload as a whole: two frames
        // in, two frames' worth of samples out. Dropping the repeat is the
        // caller's job (RedundancyDedup), not the codec's.
        let decoded = receiver.decode(&second).expect("a bundled payload decodes");
        assert_eq!(decoded.samples.len(), 320, "two frame-blocks of audio");
    }
}
