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
use codec_core::codecs::amr::sdp::AmrFmtp;
use codec_core::codecs::amr::mode::{AmrFrameType, AmrMode, AmrVariant};
use codec_core::codecs::amr::payload::{AmrPacket, AmrPayloadFrame};
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
        let payload = AmrPayloadFormat::from_negotiated(payload_type, codec_name, fmtp)
            .map_err(|error| {
                Error::Codec(CodecError::InvalidParameters {
                    details: format!("AMR fmtp: {error}"),
                })
            })?;

        if payload.codec().config().interleaving {
            return Err(Error::Codec(CodecError::InvalidParameters {
                details: "AMR interleaving is negotiated but not implemented; refusing the \
                          session rather than sending frames a conforming peer would \
                          reassemble out of order"
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
            pending_mode_request: None,
        })
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

        let coded = self.codec.encode_frame(&audio_frame.samples).map_err(|error| {
            Error::Codec(CodecError::InvalidParameters {
                details: format!("AMR encode: {error}"),
            })
        })?;

        let frame_type = match coded.kind {
            FrameKind::Speech => AmrFrameType::Speech(
                AmrMode::new(self.variant, coded.mode).unwrap_or(self.mode),
            ),
            FrameKind::ComfortNoise => AmrFrameType::Sid(self.variant),
            FrameKind::NoData => AmrFrameType::NoData,
            FrameKind::Lost => AmrFrameType::SpeechLost,
        };
        let frame = AmrPayloadFrame::new(frame_type, true, coded.data).map_err(|error| {
            Error::Codec(CodecError::InvalidParameters {
                details: format!("AMR frame: {error}"),
            })
        })?;

        self.payload
            .codec()
            .pack(&AmrPacket::single(frame))
            .map_err(|error| {
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
            let coded = match frame.frame_type {
                AmrFrameType::Speech(mode) => CodedFrame {
                    kind: FrameKind::Speech,
                    mode: mode.index(),
                    data: frame.data.clone(),
                    quality_ok: frame.quality_ok,
                },
                AmrFrameType::Sid(_) => CodedFrame {
                    kind: FrameKind::ComfortNoise,
                    mode: self.mode.index(),
                    data: frame.data.clone(),
                    quality_ok: frame.quality_ok,
                },
                AmrFrameType::NoData => CodedFrame {
                    kind: FrameKind::NoData,
                    mode: self.mode.index(),
                    data: Vec::new(),
                    quality_ok: true,
                },
                AmrFrameType::SpeechLost => CodedFrame {
                    kind: FrameKind::Lost,
                    mode: self.mode.index(),
                    data: Vec::new(),
                    quality_ok: false,
                },
            };
            let pcm = self.codec.decode_frame(&coded).map_err(|error| {
                Error::Codec(CodecError::InvalidParameters {
                    details: format!("AMR decode: {error}"),
                })
            })?;
            samples.extend_from_slice(&pcm);
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
        assert!(!variants.is_empty(), "this module needs at least one variant");

        for (name, samples, rate) in variants {
            let mut codec = AmrAdapter::new(96, name, None).expect("constructs");
            assert_eq!(codec.frame_samples(), samples, "{name} frame size");
            assert_eq!(codec.clock_rate(), rate, "{name} clock rate");

            let payload = codec.encode(&pcm(samples, rate)).expect("encodes");
            assert!(!payload.is_empty(), "{name}: an encoded frame is never empty");

            let decoded = codec.decode(&payload).expect("decodes");
            assert_eq!(decoded.samples.len(), samples, "{name} decoded length");
            assert_eq!(decoded.sample_rate, rate, "{name} decoded rate");
            assert!(
                decoded.samples.iter().any(|&s| s != 0),
                "{name}: the round trip produced silence"
            );
        }
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

        let mut deferred =
            AmrAdapter::new(96, "AMR", Some("mode-set=0,4,7; mode-change-period=2"))
                .expect("constructs");
        let mut prompt = AmrAdapter::new(96, "AMR", Some("mode-set=0,4,7")).expect("constructs");

        for codec in [&mut deferred, &mut prompt] {
            assert_ne!(codec.mode.index(), 4, "vacuous if it starts at the first target");
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
        assert_eq!(deferred.mode.index(), 0, "the second frame-block should allow it");
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
        assert_eq!(codec.mode.index(), 0, "an unsatisfiable request must be ignored");

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
}
