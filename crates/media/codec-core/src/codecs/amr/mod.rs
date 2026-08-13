//! AMR-NB and AMR-WB codecs (3GPP TS 26.090 / TS 26.190, ITU-T G.722.2).
//!
//! # Status
//!
//! **All four codec paths are bit-exact against the normative references.**
//! Both decoders reproduce the reference decoders sample for sample — AMR-WB
//! against TS 26.173 at all nine rates, AMR-NB against TS 26.073 at all eight —
//! and both encoders produce a byte-identical bitstream at every rate.
//!
//! Concealment covers both damaged frames and lost ones, on both variants.
//!
//! DTX is complete in both directions for both variants and reachable from
//! here: configure `dtx` and [`VariableRateCodec::encode_frame`] returns
//! comfort noise and gaps on the reference's own schedule (narrowband driven
//! by the bit-exact VAD1 port in `nb/enc/vad.rs`), while
//! [`VariableRateCodec::decode_frame`] accepts them.
//!
//! See `docs/AMR_IMPLEMENTATION_PLAN.md` for the phased plan and
//! `docs/AMR_IMPLEMENTATION_STATUS.md` for current progress.
//!
//! # Structure
//!
//! [`mode`] carries the codec modes, frame types and mode-set negotiation
//! logic — the parts that are pure specification data and are needed by the RTP
//! payload format and SDP layers well before any signal processing.
//!
//! # Two decode interfaces, and why
//!
//! [`AudioCodec::decode`] takes bare bytes with nowhere to say what mode they
//! are in. For AMR that would normally be fatal, since the mode is an input
//! rather than something derivable from the payload — but within one variant
//! every speech mode has a distinct frame length, so the length identifies the
//! mode unambiguously. That is what this implementation does, and it errors
//! rather than guessing when no mode matches.
//!
//! The length trick does *not* work across variants — an AMR-NB 6.70 frame and
//! an AMR-WB 6.60 frame are both 17 bytes — which is why the codec's variant is
//! fixed at construction rather than inferred. Nor can a length express a lost
//! frame or a SID frame distinct from "no bytes".
//! [`VariableRateCodec::decode_frame`] carries all of that explicitly and is
//! the interface the RTP path should use.

use crate::error::{CodecError, Result};
use crate::types::{
    AudioCodec, CodecConfig, CodecInfo, CodecType, CodedFrame, FrameKind, VariableRateCodec,
};

mod bits;
/// The normative 3GPP sequences, when the environment names them.
#[cfg(test)]
mod conformance;
pub mod mode;
pub mod nb;
pub mod payload;
/// Where the Apache-2.0 oracles agree with the normative references.
#[cfg(test)]
mod qualification;
/// Long-run stability: mode churn, DTX and loss over a full call.
#[cfg(test)]
mod soak;
/// The IF2 interface format (TS 26.101 Annex B / TS 26.201).
pub mod interface_format;
/// RFC 4867 §4.4.1 interleaving: undoing it on the receive side.
pub mod interleave;
pub mod rate;
/// RFC 4867 `max-red` redundancy: scheduling repeats and dropping them again.
pub mod redundancy;
pub mod sdp;
pub mod sid_cadence;
pub mod storage;

#[cfg(feature = "amr-wb")]
pub mod wb;

pub use mode::{AmrFrameType, AmrMode, AmrModeSet, AmrVariant};
pub use payload::{
    AmrInterleaving, AmrPacket, AmrPayloadCodec, AmrPayloadConfig, AmrPayloadFrame,
};
pub use rate::{CmrDamper, ModeChangePolicy};
pub use sdp::{AmrCapabilities, AmrFmtp};
pub use storage::AmrStorageReader;

/// AMR-NB / AMR-WB codec.
///
/// Both directions, every rate, DTX and comfort noise in both variants, plus
/// concealment of damaged and lost frames. A variant whose feature is not
/// compiled in fails with [`CodecError::FeatureNotEnabled`] naming what it
/// needs, rather than returning silence — a codec that quietly produces wrong
/// audio is far harder to diagnose than one that refuses.
pub struct AmrCodec {
    variant: AmrVariant,
    mode_set: AmrModeSet,
    current_mode: AmrMode,
    /// The negotiated `mode-change-period` and `mode-change-neighbor`, as a
    /// state machine rather than two remembered numbers.
    ///
    /// Both parameters were parsed and modelled and honoured by nothing: a
    /// peer that negotiated `mode-change-period=2` got mode changes on every
    /// frame, and one that negotiated `mode-change-neighbor=1` got arbitrary
    /// jumps. Neither is audible on its own — the frames still decode — which
    /// is why it went unnoticed.
    rate_policy: rate::ModeChangePolicy,
    octet_align: bool,
    dtx: bool,
    /// Which comfort-noise frames are actually transmitted. Shared shape
    /// between the variants; see [`sid_cadence`].
    cadence: sid_cadence::SidCadence,
    /// The decoder for this instance's variant.
    ///
    /// Boxed because each carries several kilobytes of filter history and
    /// `AmrCodec` is otherwise a handful of words that callers move freely.
    decoder: Decoder,
    /// The encoder for this instance's variant. Boxed for the same reason, and
    /// separate from the decoder because a codec instance is full duplex: the
    /// two directions carry unrelated state and must not share any.
    encoder: Encoder,
}

impl Decoder {
    /// The decoder a variant needs, or [`Decoder::Absent`] where its feature
    /// is off.
    fn for_variant(variant: AmrVariant) -> Self {
        match variant {
            #[cfg(feature = "amr-nb")]
            AmrVariant::NarrowBand => Self::NarrowBand(Box::default()),
            #[cfg(not(feature = "amr-nb"))]
            AmrVariant::NarrowBand => Self::Absent,
            #[cfg(feature = "amr-wb")]
            AmrVariant::WideBand => Self::WideBand(Box::default()),
            #[cfg(not(feature = "amr-wb"))]
            AmrVariant::WideBand => Self::Absent,
        }
    }
}

/// The variant-specific encoder, or none where the feature is off.
enum Encoder {
    #[cfg(feature = "amr-nb")]
    NarrowBand(Box<nb::enc::encoder::NbEncoder>),
    #[cfg(feature = "amr-wb")]
    WideBand(Box<wb::enc::encoder::WbEncoder>),
    /// The variant's feature is not enabled in this build.
    #[cfg_attr(all(feature = "amr-nb", feature = "amr-wb"), allow(dead_code))]
    Absent,
}

impl Encoder {
    /// The encoder a variant needs, or [`Encoder::Absent`] where its feature is
    /// off.
    fn for_variant(variant: AmrVariant) -> Self {
        match variant {
            #[cfg(feature = "amr-nb")]
            AmrVariant::NarrowBand => Self::NarrowBand(Box::default()),
            #[cfg(not(feature = "amr-nb"))]
            AmrVariant::NarrowBand => Self::Absent,
            #[cfg(feature = "amr-wb")]
            AmrVariant::WideBand => Self::WideBand(Box::default()),
            #[cfg(not(feature = "amr-wb"))]
            AmrVariant::WideBand => Self::Absent,
        }
    }
}

/// The variant-specific decoder, or none where the feature is off.
enum Decoder {
    #[cfg(feature = "amr-nb")]
    NarrowBand(Box<nb::decoder::Decoder>),
    #[cfg(feature = "amr-wb")]
    WideBand(Box<wb::decoder::Decoder>),
    /// The variant's feature is not enabled in this build.
    ///
    /// Unreachable when both features are on, which is how the crate is
    /// tested; it exists so a single-variant build still constructs, reports
    /// its configuration and refuses to decode with a message that says why.
    #[cfg_attr(all(feature = "amr-nb", feature = "amr-wb"), allow(dead_code))]
    Absent,
}

impl AmrCodec {
    /// Create an AMR codec from a codec-core configuration.
    ///
    /// # Errors
    ///
    /// Returns an error when the configuration is not a valid AMR
    /// configuration: wrong codec type, an unsupported sample rate or channel
    /// count, or a `mode-set` containing indices outside the variant's range.
    pub fn new(config: &CodecConfig) -> Result<Self> {
        config.validate()?;

        let variant = match config.codec_type {
            CodecType::AmrNb => AmrVariant::NarrowBand,
            CodecType::AmrWb => AmrVariant::WideBand,
            other => {
                return Err(CodecError::invalid_config(format!(
                    "{other} is not an AMR codec type"
                )))
            }
        };

        let params = &config.parameters.amr;
        // A zero mask means "all modes", matching an absent SDP mode-set.
        let mode_set = if params.mode_set == 0 {
            AmrModeSet::all(variant)
        } else {
            AmrModeSet::from_indices(variant, &params.mode_set_indices())?
        };

        // Start at the highest permitted mode; a peer can move us down with a
        // codec mode request. The set is non-empty by construction, so the
        // fallback here is unreachable in practice.
        let current_mode = mode_set
            .highest()
            .ok_or_else(|| CodecError::invalid_config("AMR mode-set resolved to no modes"))?;

        let rate_policy = rate::ModeChangePolicy::new(
            mode_set.clone(),
            params.mode_change_period,
            params.mode_change_neighbor,
            current_mode,
        )?;

        Ok(Self {
            variant,
            mode_set,
            current_mode,
            rate_policy,
            octet_align: params.requires_octet_align(),
            dtx: params.dtx,
            cadence: sid_cadence::SidCadence::new(variant),
            decoder: Decoder::for_variant(variant),
            encoder: {
                let mut encoder = Encoder::for_variant(variant);
                // The encoder's own DTX switch, distinct from this codec's:
                // the hangover counter runs either way because 23.85 kbit/s
                // reads it, but only a codec configured for DTX may have a
                // frame turned into comfort noise.
                #[cfg(feature = "amr-wb")]
                if let Encoder::WideBand(wide) = &mut encoder {
                    wide.set_allow_dtx(params.dtx);
                }
                // Narrowband's switch does more: with DTX off the reference
                // does not run VAD1 at all, and the open-loop stage's four
                // detector hooks are skipped with it.
                #[cfg(feature = "amr-nb")]
                if let Encoder::NarrowBand(narrow) = &mut encoder {
                    narrow.set_allow_dtx(params.dtx);
                }
                encoder
            },
        })
    }

    /// The speech mode whose frame is exactly `len` bytes, if any.
    ///
    /// Within a variant the eight (or nine) speech modes have distinct
    /// octet-aligned lengths, so this is a bijection rather than a heuristic.
    /// It does not hold *across* variants — 12.2 kbit/s narrowband and
    /// 8.85 kbit/s wideband are both 31 bytes — which is why it is a method on
    /// an instance whose variant is already fixed.
    fn mode_for_frame_len(&self, len: usize) -> Option<AmrMode> {
        AmrMode::all(self.variant)
            .into_iter()
            .find(|m| m.octet_aligned_bytes() == len)
    }

    /// Decode one frame, given its mode explicitly.
    ///
    /// Split out so both decode interfaces share it: the difference between
    /// them is only how the mode is established.
    fn decode_speech(&mut self, mode: AmrMode, data: &[u8]) -> Result<Vec<i16>> {
        self.decode_frame_bits(mode, data, true)
    }

    /// Decode one frame's bits, honouring the RFC 4867 quality bit.
    ///
    /// `quality_ok == false` is the Q bit clear: the bits arrived but the
    /// transport does not vouch for them. That is *not* the same as a lost
    /// frame — the reference still reads the codebook bits and only conceals
    /// the parameters it will not trust — so it is a flag rather than a
    /// separate entry point.
    fn decode_frame_bits(
        &mut self,
        mode: AmrMode,
        data: &[u8],
        quality_ok: bool,
    ) -> Result<Vec<i16>> {
        let short = || {
            CodecError::decoding_failed(format!(
                "{} {} kbit/s frame needs {} bytes, got {}",
                self.variant,
                f64::from(mode.bitrate()) / 1000.0,
                mode.octet_aligned_bytes(),
                data.len()
            ))
        };

        match &mut self.decoder {
            #[cfg(feature = "amr-nb")]
            Decoder::NarrowBand(decoder) => {
                // Through `decode_typed`, not `decode_parameters`, even though
                // this frame is certainly speech. `rx_dtx_handler` runs on
                // *every* frame and the backward analysis is fed from every
                // decoded speech frame, so a speech path that skipped it would
                // enter the next silence with an empty history and a stale
                // hangover count. Measured: 1928 of 24000 samples wrong at
                // 4.75 kbit/s, all of them inside the comfort noise.
                let rx = if quality_ok {
                    nb::dtx::RxFrameType::SpeechGood
                } else {
                    nb::dtx::RxFrameType::SpeechBad
                };
                let params = nb::bitstream::parse(mode.index(), data).ok_or_else(short)?;
                Ok(decoder.decode_typed(rx, mode.index(), &params).to_vec())
            }
            #[cfg(feature = "amr-wb")]
            Decoder::WideBand(decoder) => {
                let quality = if quality_ok {
                    wb::gain::FrameQuality::Good
                } else {
                    wb::gain::FrameQuality::Bad
                };
                decoder
                    .decode_frame(mode, data, quality)
                    .map(Vec::from)
                    .ok_or_else(short)
            }
            Decoder::Absent => Err(CodecError::feature_not_enabled(format!(
                "{} decoding needs its cargo feature enabled",
                self.variant
            ))),
        }
    }

    /// Encode one frame of PCM in `mode`.
    ///
    /// `samples` must be exactly one frame — 160 at 8 kHz, 320 at 16 kHz.
    /// A short or long buffer is an error rather than something to pad or
    /// truncate: the encoder's analysis window reaches across frame
    /// boundaries, so a caller that mis-frames the audio would get plausible
    /// output that drifts from what the far end reconstructs.
    fn encode_speech(&mut self, mode: AmrMode, samples: &[i16]) -> Result<Vec<u8>> {
        let wanted = self.variant.frame_samples();
        if samples.len() != wanted {
            return Err(CodecError::encoding_failed(format!(
                "{} takes exactly {wanted} samples per frame, got {}",
                self.variant,
                samples.len()
            )));
        }

        match &mut self.encoder {
            #[cfg(feature = "amr-nb")]
            Encoder::NarrowBand(encoder) => {
                let mut frame = [0i16; 160];
                frame.copy_from_slice(samples);
                let rate = nb::enc::encoder::Rate::from_index(mode.index())
                    .ok_or_else(|| CodecError::encoding_failed("not an AMR-NB speech mode"))?;
                Ok(encoder.encode_frame(&frame, rate))
            }
            #[cfg(feature = "amr-wb")]
            Encoder::WideBand(encoder) => {
                let mut frame = [0i16; 320];
                frame.copy_from_slice(samples);
                let rate = wb::enc::encoder::Rate::from_index(mode.index())
                    .ok_or_else(|| CodecError::encoding_failed("not an AMR-WB speech mode"))?;
                Ok(encoder.encode_frame(&frame, rate))
            }
            Encoder::Absent => Err(CodecError::feature_not_enabled(format!(
                "{} encoding needs its cargo feature enabled",
                self.variant
            ))),
        }
    }

    /// The variant this instance codes.
    #[must_use]
    pub const fn variant(&self) -> AmrVariant {
        self.variant
    }

    /// One frame, without the policy bookkeeping — see
    /// [`VariableRateCodec::encode_frame`].
    fn encode_one_frame(&mut self, samples: &[i16]) -> Result<CodedFrame> {
        let mode = self.current_mode;
        if !self.dtx {
            let data = self.encode_speech(mode, samples)?;
            return Ok(CodedFrame::speech(mode.index(), data));
        }

        let wanted = self.variant.frame_samples();
        if samples.len() != wanted {
            return Err(CodecError::encoding_failed(format!(
                "{} takes exactly {wanted} samples per frame, got {}",
                self.variant,
                samples.len()
            )));
        }

        match &mut self.encoder {
            #[cfg(feature = "amr-wb")]
            Encoder::WideBand(encoder) => {
                let mut frame = [0i16; 320];
                frame.copy_from_slice(samples);
                let rate = wb::enc::encoder::Rate::from_index(mode.index())
                    .ok_or_else(|| CodecError::encoding_failed("not an AMR-WB speech mode"))?;
                let (comfort_noise, mut data) = encoder.encode_frame_typed(&frame, rate);
                if !comfort_noise {
                    self.cadence.next(false, mode);
                    return Ok(CodedFrame::speech(mode.index(), data));
                }
                // The encoder builds a SID on every comfort-noise frame; the
                // cadence decides which are actually sent. Most become gaps.
                match self.cadence.next(true, mode) {
                    AmrFrameType::Sid(_) => {
                        let update = self.cadence.last_sid_was_an_update();
                        wb::bitstream::finish_sid_payload(&mut data, update, mode.index());
                        if !update {
                            wb::bitstream::blank_sid_first(&mut data);
                        }
                        Ok(CodedFrame::comfort_noise(data))
                    }
                    _ => Ok(CodedFrame::no_data()),
                }
            }
            #[cfg(feature = "amr-nb")]
            Encoder::NarrowBand(encoder) => {
                let mut frame = [0i16; 160];
                frame.copy_from_slice(samples);
                let rate = nb::enc::encoder::Rate::from_index(mode.index())
                    .ok_or_else(|| CodecError::encoding_failed("not an AMR-NB speech mode"))?;
                let (comfort_noise, mut data) = encoder.encode_frame_typed(&frame, rate);
                if !comfort_noise {
                    self.cadence.next(false, mode);
                    return Ok(CodedFrame::speech(mode.index(), data));
                }
                match self.cadence.next(true, mode) {
                    AmrFrameType::Sid(_) => {
                        // No blanking on a SID_FIRST: narrowband transmits
                        // the description on both SID types. See
                        // `nb::bitstream::finish_sid_payload`.
                        let update = self.cadence.last_sid_was_an_update();
                        nb::bitstream::finish_sid_payload(&mut data, update, mode.index());
                        Ok(CodedFrame::comfort_noise(data))
                    }
                    _ => Ok(CodedFrame::no_data()),
                }
            }
            Encoder::Absent => Err(CodecError::feature_not_enabled(
                "no AMR encoder is compiled in",
            )),
        }
    }

    /// The negotiated mode set.
    #[must_use]
    pub const fn mode_set(&self) -> &AmrModeSet {
        &self.mode_set
    }

    /// Whether octet-aligned framing is in use.
    #[must_use]
    pub const fn octet_aligned(&self) -> bool {
        self.octet_align
    }

    /// Turn discontinuous transmission on or off.
    ///
    /// Local policy, not a negotiated parameter: RFC 4867 defines no fmtp for
    /// DTX and a sender may use it or not without telling the peer, because
    /// every conforming receiver has to handle SID and `NO_DATA` frames
    /// regardless. So this is a runtime switch rather than something read out
    /// of an answer.
    ///
    /// It reaches the *encoder's* own switch too, which is a different thing:
    /// wideband's hangover counter runs either way because 23.85 kbit/s reads
    /// it, and narrowband skips VAD1 entirely when this is off.
    pub fn set_allow_dtx(&mut self, allow: bool) {
        self.dtx = allow;
        #[cfg(feature = "amr-wb")]
        if let Encoder::WideBand(wide) = &mut self.encoder {
            wide.set_allow_dtx(allow);
        }
        #[cfg(feature = "amr-nb")]
        if let Encoder::NarrowBand(narrow) = &mut self.encoder {
            narrow.set_allow_dtx(allow);
        }
    }

    /// Whether discontinuous transmission is enabled.
    #[must_use]
    pub const fn dtx_enabled(&self) -> bool {
        self.dtx
    }

    /// The mode the encoder would use for the next frame.
    #[must_use]
    pub const fn mode(&self) -> AmrMode {
        self.current_mode
    }

}

impl AudioCodec for AmrCodec {
    fn encode(&mut self, samples: &[i16]) -> Result<Vec<u8>> {
        // The currently selected mode, which a peer's codec mode request may
        // have moved. `encode_frame` is the interface that reports which mode
        // was actually used.
        self.encode_speech(self.current_mode, samples)
    }

    fn decode(&mut self, data: &[u8]) -> Result<Vec<i16>> {
        let mode = self.mode_for_frame_len(data.len()).ok_or_else(|| {
            CodecError::decoding_failed(format!(
                "{} byte frame matches no {} speech mode; use \
                 VariableRateCodec::decode_frame to say the mode explicitly",
                data.len(),
                self.variant
            ))
        })?;
        self.decode_speech(mode, data)
    }

    fn info(&self) -> CodecInfo {
        CodecInfo {
            name: self.variant.sdp_name(),
            sample_rate: self.variant.sample_rate(),
            channels: 1,
            bitrate: self.current_mode.bitrate(),
            frame_size: self.variant.frame_samples(),
            // AMR always uses a dynamic payload type.
            payload_type: None,
        }
    }

    fn reset(&mut self) -> Result<()> {
        // Mode selection deliberately survives a reset: it is negotiated state,
        // not stream state, and silently renegotiating the bit rate on a
        // discontinuity would be a much harder bug to find than a wrong sample.
        self.decoder = Decoder::for_variant(self.variant);
        self.encoder = Encoder::for_variant(self.variant);
        Ok(())
    }

    fn frame_size(&self) -> usize {
        self.variant.frame_samples()
    }
}

impl VariableRateCodec for AmrCodec {
    fn allowed_modes(&self) -> Vec<u8> {
        self.mode_set
            .modes()
            .iter()
            .map(|mode| mode.index())
            .collect()
    }

    fn current_mode(&self) -> u8 {
        self.current_mode.index()
    }

    /// Request a rate change, honouring the negotiated change policy.
    ///
    /// A mode outside the negotiated set is an error, because RFC 4867 says
    /// such frames "MUST NOT be sent in any RTP payload" — that is a
    /// configuration mistake, not a request to decline.
    ///
    /// A mode inside the set may still not take effect this frame: with
    /// `mode-change-period=2` a change is only permitted on every second
    /// frame-block, and with `mode-change-neighbor=1` a distant target is
    /// approached one step at a time. Both return `Ok`, and the mode actually
    /// in effect is [`current_mode`](Self::current_mode) — a caller that needs
    /// to know whether the request landed should read it back rather than
    /// assume.
    fn set_mode(&mut self, mode: u8) -> Result<()> {
        let requested = AmrMode::new(self.variant, mode)?;
        if !self.mode_set.contains(requested) {
            return Err(CodecError::invalid_config(format!(
                "{} mode {mode} is outside the negotiated mode-set ({})",
                self.variant,
                self.mode_set.to_sdp_value()
            )));
        }
        self.current_mode = self.rate_policy.request(requested);
        Ok(())
    }

    fn encode_frame(&mut self, samples: &[i16]) -> Result<CodedFrame> {
        let coded = self.encode_one_frame(samples);
        // One frame-block encoded, whatever it turned into. The change-period
        // counter measures frames, not successful ones -- a DTX gap still
        // occupies its slot on the wire.
        self.rate_policy.advance();
        coded
    }

    fn decode_frame(&mut self, frame: &CodedFrame) -> Result<Vec<i16>> {
        match frame.kind {
            FrameKind::Speech => {
                let mode = AmrMode::new(self.variant, frame.mode)?;
                self.decode_frame_bits(mode, &frame.data, frame.quality_ok)
            }
            // A frame the *transport* reports as missing -- a sequence-number
            // gap, or wideband's SPEECH_LOST. The caller supplies the mode the
            // stream was last using, because a lost frame has none of its own
            // and only the receiver knows what the sequence numbers imply.
            //
            // Distinct from an in-band NO_DATA frame, which arrives intact and
            // says the sender deliberately transmitted nothing. That is a DTX
            // statement about comfort noise, not a loss, and it is refused
            // below rather than concealed.
            FrameKind::Lost => match &mut self.decoder {
                #[cfg(feature = "amr-wb")]
                Decoder::WideBand(decoder) => {
                    let mode = AmrMode::new(self.variant, frame.mode)
                        .unwrap_or(self.current_mode);
                    decoder
                        .decode_frame(mode, &[], wb::gain::FrameQuality::Unusable)
                        .map(Vec::from)
                        .ok_or_else(|| {
                            CodecError::decoding_failed("AMR-WB concealment produced no frame")
                        })
                }
                #[cfg(feature = "amr-nb")]
                Decoder::NarrowBand(decoder) => {
                    // Narrowband has no `SPEECH_LOST`: the reference maps a
                    // missing frame to `RX_NO_DATA`, and what that *means*
                    // depends on the DTX state. Mid-talk-spurt it is a hole to
                    // conceal; mid-silence it is the encoder saying nothing,
                    // and comfort noise is the right answer rather than
                    // extrapolated speech. `decode_typed` makes that
                    // distinction, which `conceal_lost_frame` cannot.
                    let mode = AmrMode::new(self.variant, frame.mode)
                        .unwrap_or(self.current_mode);
                    Ok(decoder
                        .decode_typed(nb::dtx::RxFrameType::NoData, mode.index(), &[])
                        .to_vec())
                }
                Decoder::Absent => Err(CodecError::feature_not_enabled(
                    "no AMR decoder is compiled in",
                )),
            },
            // Comfort noise, and the gaps between updates. A SID and a
            // NO_DATA are different statements -- one renews the noise
            // description, the other says nothing changed -- and the decoder
            // needs to know which, so the frame kind carries it rather than
            // the payload length.
            FrameKind::ComfortNoise | FrameKind::NoData => match &mut self.decoder {
                #[cfg(feature = "amr-wb")]
                Decoder::WideBand(decoder) => {
                    let rx = if frame.kind == FrameKind::NoData {
                        wb::dtx::RxFrameType::NoData
                    } else if !frame.quality_ok {
                        wb::dtx::RxFrameType::SidBad
                    } else if wb::bitstream::sid_is_update(&frame.data) {
                        wb::dtx::RxFrameType::SidUpdate
                    } else {
                        wb::dtx::RxFrameType::SidFirst
                    };
                    // The mode selects the high-band branch, so it has to be
                    // the mode the *SID itself* names -- read out of its own
                    // mode-indication field. A receiver's transmit rate is an
                    // unrelated number, and using it made the same SID bytes
                    // decode to different comfort noise depending on what this
                    // end happened to be sending.
                    //
                    // A gap carries no SID and no indication, so there the
                    // caller's mode stands.
                    let mode_index = if frame.kind == FrameKind::NoData {
                        AmrMode::new(self.variant, frame.mode)
                            .unwrap_or(self.current_mode)
                            .index()
                    } else {
                        wb::bitstream::sid_mode_indication(&frame.data).unwrap_or_else(|| {
                            AmrMode::new(self.variant, frame.mode)
                                .unwrap_or(self.current_mode)
                                .index()
                        })
                    };
                    decoder
                        .decode_comfort_noise(rx, &frame.data, mode_index)
                        .map(Vec::from)
                        .ok_or_else(|| {
                            CodecError::decoding_failed(
                                "AMR-WB comfort-noise decode produced no frame",
                            )
                        })
                }
                #[cfg(feature = "amr-nb")]
                Decoder::NarrowBand(decoder) => {
                    // A SID's own payload says which of the three it is, and
                    // which speech mode the encoder had been using; a gap says
                    // nothing and takes the mode from the caller.
                    let (rx, mode_index, params) = if frame.kind == FrameKind::NoData {
                        let mode = AmrMode::new(self.variant, frame.mode)
                            .unwrap_or(self.current_mode);
                        (nb::dtx::RxFrameType::NoData, mode.index(), Vec::new())
                    } else {
                        let header = nb::bitstream::parse_sid_header(&frame.data)
                            .ok_or_else(|| {
                                CodecError::decoding_failed(
                                    "an AMR-NB SID frame is five octets",
                                )
                            })?;
                        let kind = if !frame.quality_ok {
                            nb::dtx::RxFrameType::SidBad
                        } else if header.update {
                            nb::dtx::RxFrameType::SidUpdate
                        } else {
                            nb::dtx::RxFrameType::SidFirst
                        };
                        // The description is read with the SID's own 35-bit
                        // layout, never the speech mode's. A SID_FIRST carries
                        // no description at all.
                        let params = if header.update || !frame.quality_ok {
                            nb::bitstream::parse(8, &frame.data).ok_or_else(|| {
                                CodecError::decoding_failed("malformed AMR-NB SID payload")
                            })?
                        } else {
                            Vec::new()
                        };
                        (kind, header.mode_index, params)
                    };
                    Ok(decoder.decode_typed(rx, mode_index, &params).to_vec())
                }
                Decoder::Absent => Err(CodecError::feature_not_enabled(
                    "no AMR decoder is compiled in",
                )),
            },
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::SampleRate;

    #[test]
    fn constructs_from_default_configs() {
        let nb = AmrCodec::new(&CodecConfig::amr_nb()).unwrap();
        assert_eq!(nb.variant(), AmrVariant::NarrowBand);
        assert_eq!(nb.frame_size(), 160);
        assert_eq!(nb.info().sample_rate, 8_000);
        assert_eq!(nb.info().name, "AMR");
        assert_eq!(nb.info().payload_type, None);

        let wb = AmrCodec::new(&CodecConfig::amr_wb()).unwrap();
        assert_eq!(wb.variant(), AmrVariant::WideBand);
        assert_eq!(wb.frame_size(), 320);
        assert_eq!(wb.info().sample_rate, 16_000);
        assert_eq!(wb.info().name, "AMR-WB");
    }

    #[test]
    fn defaults_to_bandwidth_efficient_framing() {
        // RFC 4867's default, and the one that trips implementations that
        // assume octet-aligned.
        let wb = AmrCodec::new(&CodecConfig::amr_wb()).unwrap();
        assert!(!wb.octet_aligned());

        let aligned = AmrCodec::new(&CodecConfig::amr_wb().with_amr_octet_align(true)).unwrap();
        assert!(aligned.octet_aligned());
    }

    #[test]
    fn starts_at_the_highest_permitted_mode() {
        let wb = AmrCodec::new(&CodecConfig::amr_wb()).unwrap();
        assert_eq!(wb.current_mode(), 8);

        let restricted =
            AmrCodec::new(&CodecConfig::amr_wb().with_amr_mode_set(&[0, 1, 2])).unwrap();
        assert_eq!(restricted.current_mode(), 2);
    }

    #[test]
    fn mode_set_is_enforced_on_explicit_selection() {
        let mut wb =
            AmrCodec::new(&CodecConfig::amr_wb().with_amr_mode_set(&[0, 2, 4])).unwrap();
        assert_eq!(wb.allowed_modes(), vec![0, 2, 4]);

        wb.set_mode(0).unwrap();
        assert_eq!(wb.current_mode(), 0);

        // In range for the variant but outside the negotiated set.
        assert!(wb.set_mode(3).is_err());
        assert_eq!(wb.current_mode(), 0, "a rejected change must not take effect");

        // Outside the variant's range entirely.
        assert!(wb.set_mode(9).is_err());
    }

    #[test]
    fn peer_mode_requests_are_clamped_not_errors() {
        // A CMR arrives from the network, so an out-of-set value is a peer bug,
        // not ours: ignore it rather than failing the call.
        let mut wb =
            AmrCodec::new(&CodecConfig::amr_wb().with_amr_mode_set(&[0, 2, 4])).unwrap();
        wb.set_mode(4).unwrap();

        wb.apply_mode_request(Some(2)).unwrap();
        assert_eq!(wb.current_mode(), 2);

        // Not in the mode-set: ignored, mode unchanged.
        wb.apply_mode_request(Some(7)).unwrap();
        assert_eq!(wb.current_mode(), 2);

        // CMR 15 / no request: unchanged.
        wb.apply_mode_request(None).unwrap();
        assert_eq!(wb.current_mode(), 2);
    }

    #[test]
    fn rejects_non_amr_and_malformed_configs() {
        assert!(AmrCodec::new(&CodecConfig::g711_pcmu()).is_err());

        // Wrong sample rate for the variant.
        let bad_rate = CodecConfig::amr_wb().with_sample_rate(SampleRate::Rate8000);
        assert!(AmrCodec::new(&bad_rate).is_err());

        // mode-set index out of range for narrowband.
        let bad_modes = CodecConfig::amr_nb().with_amr_mode_set(&[0, 8]);
        assert!(AmrCodec::new(&bad_modes).is_err());
    }

    #[test]
    fn unsupported_operations_fail_loudly_rather_than_returning_silence() {
        let mut wb = AmrCodec::new(&CodecConfig::amr_wb()).unwrap();

        // A gap arriving *during speech* is a hole to conceal, not a
        // statement about background noise -- and both variants conceal it
        // rather than refusing. This used to be an error on the wideband side,
        // which turned every `NO_DATA` following a lost `SID_FIRST` into a
        // dropped packet.
        assert_eq!(wb.decode_frame(&CodedFrame::no_data()).unwrap().len(), 320);

        // Narrowband draws that line differently, and deliberately: it has no
        // `SPEECH_LOST` frame type, so its receiver reads a gap through the
        // same state machine either way and always has an answer. What it
        // refuses instead is a malformed SID -- a comfort-noise frame that is
        // not five octets describes nothing, and decoding it would synthesise
        // noise from whatever the short buffer happened to parse as.
        #[cfg(feature = "amr-nb")]
        {
            let mut nb = AmrCodec::new(&CodecConfig::amr_nb()).unwrap();
            assert_eq!(nb.decode_frame(&CodedFrame::no_data()).unwrap().len(), 160);

            let err = nb
                .decode_frame(&CodedFrame::comfort_noise(vec![0; 3]))
                .unwrap_err();
            assert!(err.to_string().contains("five octets"), "{err}");
        }
        // A lost frame is the one gap wideband can fill.
        assert_eq!(wb.decode_frame(&CodedFrame::lost()).unwrap().len(), 320);

        // A mis-framed buffer is an error rather than something to pad: the
        // analysis window reaches across frame boundaries, so a caller that
        // mis-frames the audio would get plausible output that drifts from
        // what the far end reconstructs.
        assert!(wb.encode(&vec![0i16; 319]).is_err());
        assert!(wb.encode(&vec![0i16; 321]).is_err());

        let mut nb = AmrCodec::new(&CodecConfig::amr_nb()).unwrap();
        assert!(nb.encode(&vec![0i16; 159]).is_err());
    }

    /// Encoding through the public API produces exactly the reference
    /// bitstream, for both variants at every rate.
    ///
    /// The encoders are byte-exact in their own modules; this asserts the
    /// wiring reaches them without disturbing anything — a layer that reset
    /// state between frames, or mis-framed the audio, would still produce
    /// plausible speech and pass any weaker test.
    #[test]
    fn the_public_api_encodes_byte_exactly_at_every_rate() {
        for (variant, config, samples, input, refs) in [
            (
                AmrVariant::NarrowBand,
                CodecConfig::amr_nb(),
                160usize,
                include_bytes!("testdata/amrnb_enc_input.pcm").as_slice(),
                [
                    include_bytes!("testdata/amrnb_enc_mode0.amr").as_slice(),
                    include_bytes!("testdata/amrnb_enc_mode1.amr").as_slice(),
                    include_bytes!("testdata/amrnb_enc_mode2.amr").as_slice(),
                    include_bytes!("testdata/amrnb_enc_mode3.amr").as_slice(),
                    include_bytes!("testdata/amrnb_enc_mode4.amr").as_slice(),
                    include_bytes!("testdata/amrnb_enc_mode5.amr").as_slice(),
                    include_bytes!("testdata/amrnb_enc_mode6.amr").as_slice(),
                    include_bytes!("testdata/amrnb_enc_mode7.amr").as_slice(),
                ]
                .to_vec(),
            ),
            (
                AmrVariant::WideBand,
                CodecConfig::amr_wb(),
                320,
                include_bytes!("testdata/amrwb_enc_input.pcm").as_slice(),
                [
                    include_bytes!("testdata/amrwb_enc_mode0.amr").as_slice(),
                    include_bytes!("testdata/amrwb_enc_mode1.amr").as_slice(),
                    include_bytes!("testdata/amrwb_enc_mode2.amr").as_slice(),
                    include_bytes!("testdata/amrwb_enc_mode3.amr").as_slice(),
                    include_bytes!("testdata/amrwb_enc_mode4.amr").as_slice(),
                    include_bytes!("testdata/amrwb_enc_mode5.amr").as_slice(),
                    include_bytes!("testdata/amrwb_enc_mode6.amr").as_slice(),
                    include_bytes!("testdata/amrwb_enc_mode7.amr").as_slice(),
                    include_bytes!("testdata/amrwb_enc_mode8.amr").as_slice(),
                ]
                .to_vec(),
            ),
        ] {
            let pcm: Vec<i16> = input
                .chunks_exact(2)
                .map(|b| i16::from_le_bytes([b[0], b[1]]))
                .collect();
            let magic = variant.storage_magic();

            for (index, want) in refs.iter().enumerate() {
                let mode = u8::try_from(index).expect("mode index");
                let mut codec = AmrCodec::new(&config.clone().with_amr_mode_set(&[mode])).unwrap();
                assert_eq!(codec.current_mode(), mode);

                let mut got: Vec<u8> = magic.to_vec();
                for frame in pcm.chunks_exact(samples) {
                    got.push((mode << 3) | 0x04);
                    got.extend(codec.encode(frame).expect("frame encodes"));
                }
                assert_eq!(got.len(), want.len(), "{variant} mode {mode}: length");
                assert_eq!(&got, want, "{variant} mode {mode} is not byte-exact");
            }
        }
    }

    /// What the encoder emits, the decoder reads back — through the traits,
    /// with the mode carried rather than inferred.
    #[test]
    fn a_coded_frame_round_trips_through_both_traits() {
        let pcm: Vec<i16> = include_bytes!("testdata/amrwb_enc_input.pcm")
            .chunks_exact(2)
            .map(|b| i16::from_le_bytes([b[0], b[1]]))
            .collect();

        let mut encoder = AmrCodec::new(&CodecConfig::amr_wb()).unwrap();
        let mut decoder = AmrCodec::new(&CodecConfig::amr_wb()).unwrap();
        for frame in pcm.chunks_exact(320).take(10) {
            let coded = encoder.encode_frame(frame).expect("encodes");
            assert_eq!(coded.kind, FrameKind::Speech);
            assert_eq!(coded.mode, 8, "the default mode-set starts at the top rate");
            assert!(coded.quality_ok);
            let pcm_out = decoder.decode_frame(&coded).expect("decodes");
            assert_eq!(pcm_out.len(), 320);
        }
    }

    #[cfg(feature = "amr-nb")]
    mod narrowband {
        use super::*;
        use crate::codecs::amr::storage;

        /// Reference PCM from TS 26.073's own decoder, 8 kHz mono
        /// little-endian. AMR-NB's output is defined as 13-bit linear and the
        /// reference clears the low three bits before writing.
        fn reference_pcm(mode: u8) -> Vec<i16> {
            let raw: &[u8] = match mode {
                0 => include_bytes!("testdata/amrnb_mode0.pcm"),
                1 => include_bytes!("testdata/amrnb_mode1.pcm"),
                2 => include_bytes!("testdata/amrnb_mode2.pcm"),
                3 => include_bytes!("testdata/amrnb_mode3.pcm"),
                4 => include_bytes!("testdata/amrnb_mode4.pcm"),
                5 => include_bytes!("testdata/amrnb_mode5.pcm"),
                6 => include_bytes!("testdata/amrnb_mode6.pcm"),
                7 => include_bytes!("testdata/amrnb_mode7.pcm"),
                other => panic!("no reference PCM for mode {other}"),
            };
            raw.chunks_exact(2)
                .map(|b| i16::from_le_bytes([b[0], b[1]]))
                .collect()
        }

        fn fixture(mode: u8) -> &'static [u8] {
            match mode {
                0 => include_bytes!("testdata/amrnb_mode0.amr"),
                1 => include_bytes!("testdata/amrnb_mode1.amr"),
                2 => include_bytes!("testdata/amrnb_mode2.amr"),
                3 => include_bytes!("testdata/amrnb_mode3.amr"),
                4 => include_bytes!("testdata/amrnb_mode4.amr"),
                5 => include_bytes!("testdata/amrnb_mode5.amr"),
                6 => include_bytes!("testdata/amrnb_mode6.amr"),
                7 => include_bytes!("testdata/amrnb_mode7.amr"),
                other => panic!("no fixture for mode {other}"),
            }
        }

        /// As for wideband: `nb::decoder` is bit-exact on its own, and this
        /// asserts the public API reaches it without disturbing anything. A
        /// wiring layer that reset state between frames would still produce
        /// speech and pass any weaker test.
        #[test]
        fn the_public_api_decodes_bit_exactly_at_every_rate() {
            for mode in 0..8u8 {
                let want = reference_pcm(mode);
                let (_, frames) = storage::read(fixture(mode)).expect("fixture parses");
                assert!(!frames.is_empty(), "mode {mode} fixture is empty");

                let mut codec = AmrCodec::new(&CodecConfig::amr_nb()).unwrap();
                let mut got = Vec::with_capacity(want.len());
                for frame in &frames {
                    got.extend(codec.decode(&frame.data).expect("frame decodes"));
                }

                assert_eq!(got.len(), want.len(), "mode {mode} sample count");
                assert_eq!(got, want, "mode {mode} is not bit-exact through the API");
            }
        }

        /// A frame length can identify the mode within a variant but never the
        /// variant, so the variant has to come from the negotiation.
        ///
        /// The colliding pair is computed rather than remembered: the first
        /// version of this test asserted narrowband 12.2 and wideband 8.85 were
        /// both 31 bytes, which is simply false, and the test caught the
        /// mistake in its own premise.
        #[test]
        #[cfg(feature = "amr-wb")]
        fn a_frame_length_cannot_identify_the_variant() {
            let nb_lengths: Vec<usize> = AmrMode::all(AmrVariant::NarrowBand)
                .iter()
                .map(|m| m.octet_aligned_bytes())
                .collect();
            let collisions: Vec<(u8, u8, usize)> = AmrMode::all(AmrVariant::WideBand)
                .iter()
                .filter_map(|wb| {
                    let len = wb.octet_aligned_bytes();
                    nb_lengths.iter().position(|&n| n == len).map(|i| {
                        (u8::try_from(i).expect("mode index"), wb.index(), len)
                    })
                })
                .collect();
            assert!(
                !collisions.is_empty(),
                "if the two variants' lengths ever became disjoint,                  AudioCodec::decode could infer the variant too"
            );

            // Decoding a colliding wideband frame as narrowband succeeds and
            // produces narrowband-shaped output. That is the hazard, stated
            // rather than guarded against: only the negotiated variant can
            // resolve it.
            let (nb_mode, _wb_mode, len) = collisions[0];
            let mut nb = AmrCodec::new(&CodecConfig::amr_nb()).unwrap();
            let (_, frames) =
                storage::read(include_bytes!("testdata/amrwb_mode0.amr")).expect("parses");
            let wb_frame = frames
                .iter()
                .find(|f| f.data.len() == len)
                .expect("a wideband frame of the colliding length");
            let decoded = nb.decode(&wb_frame.data).expect("a valid narrowband length");
            assert_eq!(decoded.len(), 160, "decoded as narrowband mode {nb_mode}");
        }

        /// A damaged frame reaches concealment through the trait, and the
        /// result matches the reference for the whole erased stream.
        ///
        /// This is the RFC 4867 Q bit doing its job end to end: the packetizer
        /// reports it, `CodedFrame::quality_ok` carries it, and the decoder
        /// conceals rather than decoding bits nobody vouches for.
        #[test]
        fn a_damaged_frame_is_concealed_rather_than_refused() {
            let want: Vec<i16> = include_bytes!("testdata/amrnb_erased.pcm")
                .chunks_exact(2)
                .map(|b| i16::from_le_bytes([b[0], b[1]]))
                .collect();
            let (_, frames) =
                storage::read(include_bytes!("testdata/amrnb_erased.amr")).expect("parses");
            assert!(
                frames.iter().any(|f| !f.quality_ok),
                "the erased fixture carries no damaged frame"
            );

            let mut codec = AmrCodec::new(&CodecConfig::amr_nb()).unwrap();
            let mut got = Vec::with_capacity(want.len());
            for frame in &frames {
                let speech = CodedFrame {
                    kind: FrameKind::Speech,
                    mode: 4,
                    quality_ok: frame.quality_ok,
                    data: frame.data.clone(),
                };
                got.extend(codec.decode_frame(&speech).expect("frame decodes"));
            }
            assert_eq!(got, want, "concealment through the trait is not bit-exact");
        }

        #[test]
        fn reset_returns_the_decoder_to_its_start_state() {
            let (_, frames) = storage::read(fixture(4)).expect("fixture parses");
            let mut codec = AmrCodec::new(&CodecConfig::amr_nb()).unwrap();

            let first: Vec<i16> = frames[..4]
                .iter()
                .flat_map(|f| codec.decode(&f.data).expect("decodes"))
                .collect();
            for f in &frames[4..12] {
                codec.decode(&f.data).expect("decodes");
            }
            codec.reset().unwrap();

            let again: Vec<i16> = frames[..4]
                .iter()
                .flat_map(|f| codec.decode(&f.data).expect("decodes"))
                .collect();
            assert_eq!(first, again, "reset did not clear the decoder state");
        }

        /// The narrowband twin, and the asymmetry it removes.
        ///
        /// Wideband has had lost-frame concealment since the decoder landed;
        /// narrowband refused, because RFC 4867 gives it no `SPEECH_LOST`
        /// frame type and the reference reaches concealment by a different
        /// route — it manufactures a parameter vector first. Both streams are
        /// the same 25 frames with the same six erasures, differing only in
        /// whether each is marked damaged or absent.
        #[test]
        fn narrowband_conceals_both_bad_frame_kinds_bit_exactly() {
            for (bits, pcm, lost) in [
                (
                    include_bytes!("testdata/amrnb_erased.amr").as_slice(),
                    include_bytes!("testdata/amrnb_erased.pcm").as_slice(),
                    false,
                ),
                (
                    include_bytes!("testdata/amrnb_lost.amr").as_slice(),
                    include_bytes!("testdata/amrnb_lost.pcm").as_slice(),
                    true,
                ),
            ] {
                let want: Vec<i16> = pcm
                    .chunks_exact(2)
                    .map(|b| i16::from_le_bytes([b[0], b[1]]))
                    .collect();
                let (_, frames) = storage::read(bits).expect("fixture parses");
                let mut codec = AmrCodec::new(&CodecConfig::amr_nb()).unwrap();
                let mut got = Vec::with_capacity(want.len());

                let mut concealed = 0usize;
                for frame in &frames {
                    let input = match frame.frame_type {
                        AmrFrameType::NoData => {
                            concealed += 1;
                            CodedFrame {
                                kind: FrameKind::Lost,
                                mode: 4,
                                quality_ok: false,
                                data: Vec::new(),
                            }
                        }
                        _ => CodedFrame {
                            kind: FrameKind::Speech,
                            mode: 4,
                            quality_ok: frame.quality_ok,
                            data: frame.data.clone(),
                        },
                    };
                    got.extend(codec.decode_frame(&input).expect("frame decodes"));
                }

                // The damaged stream has no NoData frames, so only the lost
                // one takes the concealment path; asserting the count stops
                // this from passing while decoding everything as speech.
                assert_eq!(concealed, if lost { 6 } else { 0 });
                let masked: Vec<i16> = got.iter().map(|s| s & !7).collect();
                assert_eq!(
                    masked, want,
                    "{} stream is not bit-exact through the trait",
                    if lost { "lost" } else { "damaged" }
                );
            }
        }
    }

    #[cfg(feature = "amr-wb")]
    mod wideband {
        use super::*;
        use crate::codecs::amr::storage;

        /// Reference PCM from TS 26.173's own decoder, 16 kHz mono
        /// little-endian. AMR-WB's output is defined as 14-bit linear and the
        /// reference masks the low two bits before writing, so a comparison
        /// against unmasked output scores near zero for a perfectly correct
        /// decoder — which cost a full debugging session before it was noticed.
        fn reference_pcm(mode: u8) -> Vec<i16> {
            let raw: &[u8] = match mode {
                0 => include_bytes!("testdata/amrwb_mode0.pcm"),
                1 => include_bytes!("testdata/amrwb_mode1.pcm"),
                2 => include_bytes!("testdata/amrwb_mode2.pcm"),
                3 => include_bytes!("testdata/amrwb_mode3.pcm"),
                4 => include_bytes!("testdata/amrwb_mode4.pcm"),
                5 => include_bytes!("testdata/amrwb_mode5.pcm"),
                6 => include_bytes!("testdata/amrwb_mode6.pcm"),
                7 => include_bytes!("testdata/amrwb_mode7.pcm"),
                8 => include_bytes!("testdata/amrwb_mode8.pcm"),
                other => panic!("no reference PCM for mode {other}"),
            };
            raw.chunks_exact(2)
                .map(|b| i16::from_le_bytes([b[0], b[1]]))
                .collect()
        }

        fn fixture(mode: u8) -> &'static [u8] {
            match mode {
                0 => include_bytes!("testdata/amrwb_mode0.amr"),
                1 => include_bytes!("testdata/amrwb_mode1.amr"),
                2 => include_bytes!("testdata/amrwb_mode2.amr"),
                3 => include_bytes!("testdata/amrwb_mode3.amr"),
                4 => include_bytes!("testdata/amrwb_mode4.amr"),
                5 => include_bytes!("testdata/amrwb_mode5.amr"),
                6 => include_bytes!("testdata/amrwb_mode6.amr"),
                7 => include_bytes!("testdata/amrwb_mode7.amr"),
                8 => include_bytes!("testdata/amrwb_mode8.amr"),
                other => panic!("no fixture for mode {other}"),
            }
        }

        /// The decoder is bit-exact in `wb::decoder`; this asserts the *public
        /// API* reaches it without disturbing anything. A wiring layer that
        /// resets state between frames, or loses the frame boundary, would
        /// still produce speech-shaped output and pass any weaker test.
        #[test]
        fn the_public_api_decodes_bit_exactly_at_every_rate() {
            for mode in 0..9u8 {
                let want = reference_pcm(mode);
                let (_, frames) = storage::read(fixture(mode)).expect("fixture parses");
                assert!(!frames.is_empty(), "mode {mode} fixture is empty");

                let mut codec = AmrCodec::new(&CodecConfig::amr_wb()).unwrap();
                let mut got = Vec::with_capacity(want.len());
                for frame in &frames {
                    got.extend(codec.decode(&frame.data).expect("frame decodes"));
                }

                assert_eq!(got.len(), want.len(), "mode {mode} sample count");
                // The reference's 14-bit output convention, applied to ours.
                let masked: Vec<i16> = got.iter().map(|s| s & !3).collect();
                let first_bad = masked
                    .iter()
                    .zip(&want)
                    .position(|(a, b)| a != b)
                    .map(|i| format!("first differs at sample {i}: {} vs {}", masked[i], want[i]));
                assert!(first_bad.is_none(), "mode {mode}: {}", first_bad.unwrap());
            }
        }

        /// Frame length identifies the mode within a variant, so
        /// [`AudioCodec::decode`] can work at all. If two speech modes ever
        /// collided this test fails rather than one of them silently decoding
        /// as the other.
        #[test]
        fn frame_lengths_identify_the_mode_unambiguously() {
            for variant in [AmrVariant::NarrowBand, AmrVariant::WideBand] {
                let config = match variant {
                    AmrVariant::NarrowBand => CodecConfig::amr_nb(),
                    AmrVariant::WideBand => CodecConfig::amr_wb(),
                };
                let codec = AmrCodec::new(&config).unwrap();
                for mode in AmrMode::all(variant) {
                    assert_eq!(
                        codec.mode_for_frame_len(mode.octet_aligned_bytes()),
                        Some(mode),
                        "{variant} mode {} is not recoverable from its length",
                        mode.index()
                    );
                }
                assert_eq!(codec.mode_for_frame_len(0), None);
                assert_eq!(codec.mode_for_frame_len(1000), None);
            }
        }

        /// Decoding is stateful across frames — filter memories, the adaptive
        /// codebook history, the predicted gain. Reset must clear all of it, or
        /// a reused codec carries one call's tail into the next.
        #[test]
        fn reset_returns_the_decoder_to_its_start_state() {
            let (_, frames) = storage::read(fixture(2)).expect("fixture parses");
            let mut codec = AmrCodec::new(&CodecConfig::amr_wb()).unwrap();

            let first: Vec<i16> = frames[..4]
                .iter()
                .flat_map(|f| codec.decode(&f.data).expect("decodes"))
                .collect();

            // Run further, so there is real state to carry, then reset.
            for f in &frames[4..12] {
                codec.decode(&f.data).expect("decodes");
            }
            codec.reset().unwrap();

            let again: Vec<i16> = frames[..4]
                .iter()
                .flat_map(|f| codec.decode(&f.data).expect("decodes"))
                .collect();
            assert_eq!(first, again, "reset did not clear the decoder state");
        }

        /// Damaged and lost frames both reach concealment through the trait,
        /// and both match the reference for the whole stream.
        #[test]
        fn both_bad_frame_kinds_are_concealed_bit_exactly() {
            for (bits, pcm, lost) in [
                (
                    include_bytes!("testdata/amrwb_erased.amr").as_slice(),
                    include_bytes!("testdata/amrwb_erased.pcm").as_slice(),
                    false,
                ),
                (
                    include_bytes!("testdata/amrwb_lost.amr").as_slice(),
                    include_bytes!("testdata/amrwb_lost.pcm").as_slice(),
                    true,
                ),
            ] {
                let want: Vec<i16> = pcm
                    .chunks_exact(2)
                    .map(|b| i16::from_le_bytes([b[0], b[1]]))
                    .collect();
                let (_, frames) = storage::read(bits).expect("fixture parses");
                let mut codec = AmrCodec::new(&CodecConfig::amr_wb()).unwrap();
                let mut got = Vec::with_capacity(want.len());

                for frame in &frames {
                    let speech = match frame.frame_type {
                        AmrFrameType::SpeechLost => CodedFrame {
                            kind: FrameKind::Lost,
                            mode: 2,
                            quality_ok: false,
                            data: Vec::new(),
                        },
                        _ => CodedFrame {
                            kind: FrameKind::Speech,
                            mode: 2,
                            quality_ok: frame.quality_ok,
                            data: frame.data.clone(),
                        },
                    };
                    got.extend(codec.decode_frame(&speech).expect("frame decodes"));
                }

                let masked: Vec<i16> = got.iter().map(|s| s & !3).collect();
                assert_eq!(
                    masked, want,
                    "{} stream is not bit-exact through the trait",
                    if lost { "lost" } else { "damaged" }
                );
            }
        }

        /// Encoding with DTX on reproduces the reference's own file.
        ///
        /// Frame types and payloads both: the twelve SIDs the reference chose
        /// to send, at the frames it chose to send them, with the bits it put
        /// in them. This is the encoder, the VAD, the hangover and the
        /// transmit cadence agreeing with TS 26.173 end to end through the
        /// public API rather than through the encoder's own internals.
        #[test]
        fn encoding_with_dtx_reproduces_the_reference_stream() {
            let want_bits: &[u8] = include_bytes!("testdata/amrwb_dtx_mode2.amr");
            let pcm: &[u8] = include_bytes!("testdata/amrwb_dtx_input.pcm");
            let (_, want) = storage::read(want_bits).expect("fixture parses");

            let mut config = CodecConfig::amr_wb();
            config.parameters.amr.dtx = true;
            config.parameters.amr.mode_set = 1 << 2;
            let mut codec = AmrCodec::new(&config).unwrap();

            let mut speech = 0usize;
            let mut sids = 0usize;
            let mut gaps = 0usize;
            for (n, frame) in want.iter().enumerate() {
                let samples: Vec<i16> = pcm[n * 640..(n + 1) * 640]
                    .chunks_exact(2)
                    .map(|b| i16::from_le_bytes([b[0], b[1]]))
                    .collect();
                let got = codec.encode_frame(&samples).expect("frame encodes");
                match frame.frame_type {
                    AmrFrameType::Speech(_) => {
                        speech += 1;
                        assert_eq!(got.kind, FrameKind::Speech, "frame {n} kind");
                        assert_eq!(got.data, frame.data, "frame {n} payload");
                    }
                    AmrFrameType::Sid(_) => {
                        sids += 1;
                        assert_eq!(got.kind, FrameKind::ComfortNoise, "frame {n} kind");
                        assert_eq!(got.data, frame.data, "frame {n} SID payload");
                    }
                    AmrFrameType::NoData => {
                        gaps += 1;
                        assert_eq!(got.kind, FrameKind::NoData, "frame {n} kind");
                        assert!(got.data.is_empty());
                    }
                    other @ AmrFrameType::SpeechLost => {
                        panic!("unexpected {other:?} at {n}")
                    }
                }
            }
            assert_eq!((speech, sids, gaps), (77, 12, 61));
        }

        /// The narrowband encoder with DTX on, against the reference stream.
        ///
        /// Frame type for frame type *and* payload for payload. The frame-type
        /// sequence alone would pass with a right cadence over a wrong SID —
        /// which is exactly the failure a VAD wired to a constant produces —
        /// so every transmitted SID's 35 bits are compared too, STI bit and
        /// mode indication included.
        #[test]
        fn narrowband_encoding_with_dtx_reproduces_the_reference_stream() {
            for mode in 0..8u8 {
                narrowband_dtx_rate(mode);
            }
        }

        /// One rate's DTX stream, encoder side.
        fn narrowband_dtx_rate(mode: u8) {
            let want_bits: &[u8] = match mode {
                0 => include_bytes!("testdata/amrnb_dtx_mode0.amr"),
                1 => include_bytes!("testdata/amrnb_dtx_mode1.amr"),
                2 => include_bytes!("testdata/amrnb_dtx_mode2.amr"),
                3 => include_bytes!("testdata/amrnb_dtx_mode3.amr"),
                4 => include_bytes!("testdata/amrnb_dtx_mode4.amr"),
                5 => include_bytes!("testdata/amrnb_dtx_mode5.amr"),
                6 => include_bytes!("testdata/amrnb_dtx_mode6.amr"),
                _ => include_bytes!("testdata/amrnb_dtx_mode7.amr"),
            };
            let pcm: &[u8] = include_bytes!("testdata/amrnb_dtx_input.pcm");
            let (_, want) = storage::read(want_bits).expect("fixture parses");

            let mut config = CodecConfig::amr_nb();
            config.parameters.amr.dtx = true;
            config.parameters.amr.mode_set = 1 << mode;
            let mut codec = AmrCodec::new(&config).unwrap();

            let (mut speech, mut sids, mut gaps) = (0usize, 0usize, 0usize);
            for (n, frame) in want.iter().enumerate() {
                let samples: Vec<i16> = pcm[n * 320..(n + 1) * 320]
                    .chunks_exact(2)
                    .map(|b| i16::from_le_bytes([b[0], b[1]]))
                    .collect();
                let got = codec.encode_frame(&samples).expect("frame encodes");
                match frame.frame_type {
                    AmrFrameType::Speech(_) => {
                        speech += 1;
                        assert_eq!(got.kind, FrameKind::Speech, "mode {mode} frame {n} kind");
                        assert_eq!(got.data, frame.data, "mode {mode} frame {n} payload");
                    }
                    AmrFrameType::Sid(_) => {
                        sids += 1;
                        assert_eq!(got.kind, FrameKind::ComfortNoise, "mode {mode} frame {n} kind");
                        assert_eq!(got.data, frame.data, "mode {mode} frame {n} SID payload");
                    }
                    AmrFrameType::NoData => {
                        gaps += 1;
                        assert_eq!(got.kind, FrameKind::NoData, "mode {mode} frame {n} kind");
                        assert!(got.data.is_empty());
                    }
                    other @ AmrFrameType::SpeechLost => panic!("unexpected {other:?} at {n}"),
                }
            }
            assert_eq!((speech, sids, gaps), (75, 12, 63), "mode {mode} frame-type mix");
        }

        /// The narrowband DTX stream decoded through the public API,
        /// sample-exact, on all eight rates.
        ///
        /// The decoder module's own test proves the decoder; this proves the
        /// wiring above it — that a caller with nothing but `CodedFrame` can
        /// distinguish a SID, an update and a gap, and reaches the reference's
        /// own samples doing so.
        #[test]
        #[allow(clippy::similar_names)]
        fn a_narrowband_dtx_stream_is_sample_exact_through_the_trait() {
            for mode in 0..8u8 {
                let (bits, pcm): (&[u8], &[u8]) = match mode {
                    0 => (
                        include_bytes!("testdata/amrnb_dtx_mode0.amr"),
                        include_bytes!("testdata/amrnb_dtx_mode0.pcm"),
                    ),
                    1 => (
                        include_bytes!("testdata/amrnb_dtx_mode1.amr"),
                        include_bytes!("testdata/amrnb_dtx_mode1.pcm"),
                    ),
                    2 => (
                        include_bytes!("testdata/amrnb_dtx_mode2.amr"),
                        include_bytes!("testdata/amrnb_dtx_mode2.pcm"),
                    ),
                    3 => (
                        include_bytes!("testdata/amrnb_dtx_mode3.amr"),
                        include_bytes!("testdata/amrnb_dtx_mode3.pcm"),
                    ),
                    4 => (
                        include_bytes!("testdata/amrnb_dtx_mode4.amr"),
                        include_bytes!("testdata/amrnb_dtx_mode4.pcm"),
                    ),
                    5 => (
                        include_bytes!("testdata/amrnb_dtx_mode5.amr"),
                        include_bytes!("testdata/amrnb_dtx_mode5.pcm"),
                    ),
                    6 => (
                        include_bytes!("testdata/amrnb_dtx_mode6.amr"),
                        include_bytes!("testdata/amrnb_dtx_mode6.pcm"),
                    ),
                    _ => (
                        include_bytes!("testdata/amrnb_dtx_mode7.amr"),
                        include_bytes!("testdata/amrnb_dtx_mode7.pcm"),
                    ),
                };
                let want: Vec<i16> = pcm
                    .chunks_exact(2)
                    .map(|b| i16::from_le_bytes([b[0], b[1]]))
                    .collect();
                let (_, frames) = storage::read(bits).expect("fixture parses");

                let config = CodecConfig::amr_nb();
                let mut codec = AmrCodec::new(&config).unwrap();
                let (mut exact, mut total, mut kinds) = (0usize, 0usize, [0usize; 3]);

                for (n, frame) in frames.iter().enumerate() {
                    let coded = match frame.frame_type {
                        AmrFrameType::Sid(_) => {
                            kinds[1] += 1;
                            CodedFrame {
                                kind: FrameKind::ComfortNoise,
                                mode,
                                data: frame.data.clone(),
                                quality_ok: frame.quality_ok,
                            }
                        }
                        AmrFrameType::NoData => {
                            kinds[2] += 1;
                            CodedFrame {
                                kind: FrameKind::NoData,
                                mode,
                                data: Vec::new(),
                                quality_ok: true,
                            }
                        }
                        _ => {
                            kinds[0] += 1;
                            CodedFrame {
                                kind: FrameKind::Speech,
                                mode,
                                data: frame.data.clone(),
                                quality_ok: frame.quality_ok,
                            }
                        }
                    };
                    let got = codec.decode_frame(&coded).expect("frame decodes");
                    for (i, &sample) in got.iter().enumerate() {
                        let at = n * 160 + i;
                        if at >= want.len() {
                            break;
                        }
                        total += 1;
                        exact += usize::from(sample == want[at]);
                    }
                }

                assert_eq!(kinds, [75, 12, 63], "mode {mode} frame-type mix");
                assert_eq!(exact, total, "mode {mode}: {} of {total} differ", total - exact);
            }
        }

        /// `mode-change-period=2` is honoured through the public API.
        ///
        /// Both this and `mode-change-neighbor` were parsed, modelled in
        /// `rate.rs`, tested there, and called by nothing: `set_mode` assigned
        /// straight to `current_mode`. Neither is audible on its own — the
        /// frames decode either way — so only a test that drives the codec and
        /// reads the mode back catches it.
        #[test]
        #[cfg(feature = "amr-nb")]
        fn a_negotiated_change_period_delays_a_rate_change() {
            let mut config = CodecConfig::amr_nb();
            config.parameters.amr.mode_change_period = 2;
            let mut codec = AmrCodec::new(&config).unwrap();

            let start = codec.current_mode();
            assert_eq!(start, 7, "the mode set is all modes, so it opens at 12.2");

            // The first request lands: the policy starts with its interval
            // already elapsed, per RFC 4867's "the initial phase of the
            // interval is arbitrary".
            codec.set_mode(4).unwrap();
            assert_eq!(codec.current_mode(), 4);

            // The next one does not, because only one frame-block has passed.
            let frame = vec![0i16; 160];
            codec.encode_frame(&frame).unwrap();
            codec.set_mode(0).unwrap();
            assert_eq!(codec.current_mode(), 4, "a change came too soon");

            // After a second frame-block it does.
            codec.encode_frame(&frame).unwrap();
            codec.set_mode(0).unwrap();
            assert_eq!(codec.current_mode(), 0);

            // And with period 1 the same sequence changes every time, so the
            // assertion above is about the period rather than about anything
            // else in the path.
            let mut config = CodecConfig::amr_nb();
            config.parameters.amr.mode_change_period = 1;
            let mut every = AmrCodec::new(&config).unwrap();
            every.set_mode(4).unwrap();
            every.encode_frame(&frame).unwrap();
            every.set_mode(0).unwrap();
            assert_eq!(every.current_mode(), 0, "period 1 should not delay");
        }

        /// `mode-change-neighbor=1` walks rather than jumps.
        #[test]
        #[cfg(feature = "amr-nb")]
        fn a_negotiated_neighbor_restriction_steps_one_mode_at_a_time() {
            let mut config = CodecConfig::amr_nb();
            config.parameters.amr.mode_change_neighbor = true;
            let mut codec = AmrCodec::new(&config).unwrap();
            let frame = vec![0i16; 160];

            assert_eq!(codec.current_mode(), 7);
            // Asking for 0 from 7 moves one step, not seven.
            codec.set_mode(0).unwrap();
            assert_eq!(codec.current_mode(), 6);

            let mut steps = 1;
            while codec.current_mode() != 0 {
                codec.encode_frame(&frame).unwrap();
                codec.set_mode(0).unwrap();
                steps += 1;
                assert!(steps <= 8, "the walk did not converge");
            }
            assert_eq!(steps, 7, "seven single steps from mode 7 to mode 0");

            // Unrestricted, the same request arrives in one.
            let mut direct = AmrCodec::new(&CodecConfig::amr_nb()).unwrap();
            direct.set_mode(0).unwrap();
            assert_eq!(direct.current_mode(), 0);
        }

        /// A mode outside the negotiated set is still an error, not a
        /// declined request.
        #[test]
        #[cfg(feature = "amr-nb")]
        fn a_mode_outside_the_set_is_refused_rather_than_deferred() {
            let config = CodecConfig::amr_nb().with_amr_mode_set(&[0, 4]);
            let mut codec = AmrCodec::new(&config).unwrap();
            assert!(codec.set_mode(7).is_err(), "mode 7 is outside the set");
            assert!(codec.set_mode(0).is_ok());
        }

        /// A `NO_DATA` arriving mid-talk-spurt is a lost frame, not an error.
        ///
        /// The reference's own state table (`dtx.c`, above `rx_dtx_handler`)
        /// gives `RX_NO_DATA | SPEECH -> SPEECH`, and the SPEECH branch
        /// conceals. Returning an error instead drops the packet — reachable
        /// on any real network the moment the `SID_FIRST` that opens a silence
        /// is lost, after which *every* `NO_DATA` for the rest of that gap is
        /// discarded.
        ///
        /// The DTX tests never caught it because they feed complete, in-order
        /// reference streams, so the state machine is always already in DTX by
        /// the time a `NO_DATA` arrives.
        #[test]
        #[cfg(feature = "amr-wb")]
        fn a_gap_arriving_during_speech_is_concealed_rather_than_refused() {
            let bits: &[u8] = include_bytes!("testdata/amrwb_dtx_mode2.amr");
            let (_, frames) = storage::read(bits).expect("fixture parses");
            let mut codec = AmrCodec::new(&CodecConfig::amr_wb()).unwrap();

            let mut decoded = 0;
            for frame in frames.iter().take(20) {
                if let AmrFrameType::Speech(mode) = frame.frame_type {
                    codec
                        .decode_frame(&CodedFrame {
                            kind: FrameKind::Speech,
                            mode: mode.index(),
                            data: frame.data.clone(),
                            quality_ok: true,
                        })
                        .expect("speech decodes");
                    decoded += 1;
                    if decoded == 5 {
                        break;
                    }
                }
            }
            assert_eq!(decoded, 5, "the fixture did not supply five speech frames");

            let out = codec
                .decode_frame(&CodedFrame {
                    kind: FrameKind::NoData,
                    mode: 2,
                    data: Vec::new(),
                    quality_ok: true,
                })
                .expect("a gap during speech must be concealed, not refused");
            assert_eq!(out.len(), 320, "concealment produces a whole frame");
        }

        /// A SID is decoded against the mode *it* names, not against whatever
        /// this end happens to be transmitting.
        ///
        /// The mode selects the high-band branch, so the same SID bytes
        /// decoded by two receivers transmitting at different rates would
        /// otherwise produce different comfort noise. The code comment at the
        /// call site names this exact failure; the caller committed it.
        #[test]
        #[cfg(feature = "amr-wb")]
        fn comfort_noise_is_decoded_against_the_sids_own_mode() {
            let bits: &[u8] = include_bytes!("testdata/amrwb_dtx_mode2.amr");
            let (_, frames) = storage::read(bits).expect("fixture parses");
            let sid = frames
                .iter()
                .find(|f| matches!(f.frame_type, AmrFrameType::Sid(_)))
                .expect("the fixture carries a SID");

            let decode_with = |claimed_mode: u8| {
                let mut codec = AmrCodec::new(&CodecConfig::amr_wb()).unwrap();
                codec
                    .decode_frame(&CodedFrame {
                        kind: FrameKind::ComfortNoise,
                        mode: claimed_mode,
                        data: sid.data.clone(),
                        quality_ok: true,
                    })
                    .expect("comfort noise decodes")
            };

            assert_eq!(
                decode_with(0),
                decode_with(8),
                "the same SID decoded differently depending on the receiver's own rate"
            );
        }

        /// A real DTX stream, decoded entirely through the public API.
        ///
        /// The module's own test proves the decoder; this proves the wiring
        /// above it — that `FrameKind` carries enough for a caller to say
        /// which of a SID, an update and a gap arrived, and that the codec
        /// reaches the same samples the reference does through it.
        #[test]
        fn a_dtx_stream_is_sample_exact_through_the_trait() {
            let bits: &[u8] = include_bytes!("testdata/amrwb_dtx_mode2.amr");
            let want: Vec<i16> = include_bytes!("testdata/amrwb_dtx_mode2.pcm")
                .chunks_exact(2)
                .map(|b| i16::from_le_bytes([b[0], b[1]]))
                .collect();
            let (_, frames) = storage::read(bits).expect("fixture parses");
            let mut codec = AmrCodec::new(&CodecConfig::amr_wb()).unwrap();
            let mut got = Vec::with_capacity(want.len());

            let mut kinds = (0usize, 0usize, 0usize);
            for frame in &frames {
                let request = match frame.frame_type {
                    AmrFrameType::Sid(_) => {
                        kinds.1 += 1;
                        CodedFrame {
                            kind: FrameKind::ComfortNoise,
                            mode: 2,
                            quality_ok: true,
                            data: frame.data.clone(),
                        }
                    }
                    AmrFrameType::NoData => {
                        kinds.2 += 1;
                        CodedFrame {
                            kind: FrameKind::NoData,
                            mode: 2,
                            quality_ok: true,
                            data: Vec::new(),
                        }
                    }
                    _ => {
                        kinds.0 += 1;
                        CodedFrame {
                            kind: FrameKind::Speech,
                            mode: 2,
                            quality_ok: frame.quality_ok,
                            data: frame.data.clone(),
                        }
                    }
                };
                got.extend(codec.decode_frame(&request).expect("frame decodes"));
            }

            assert_eq!(kinds, (77, 12, 61), "the fixture's frame mix moved");
            let masked: Vec<i16> = got.iter().map(|s| s & !3).collect();
            assert_eq!(masked, want, "not sample-exact through the trait");
        }

        #[test]
        fn a_frame_of_the_wrong_length_is_rejected_rather_than_decoded() {
            let mut codec = AmrCodec::new(&CodecConfig::amr_wb()).unwrap();
            // One byte short of mode 8's 60. Truncation is what a damaged
            // packet looks like, and decoding it as a shorter mode would
            // produce plausible garbage.
            assert!(codec.decode(&[0u8; 59]).is_err());
            assert!(codec.decode(&[]).is_err());
        }
    }

    #[test]
    fn reset_preserves_negotiated_mode() {
        // Mode selection is negotiated state, not stream state: a
        // discontinuity must not silently renegotiate the bit rate.
        let mut wb =
            AmrCodec::new(&CodecConfig::amr_wb().with_amr_mode_set(&[0, 2, 4])).unwrap();
        wb.set_mode(0).unwrap();
        wb.reset().unwrap();
        assert_eq!(wb.current_mode(), 0);
    }
}
