//! AMR-NB and AMR-WB codecs (3GPP TS 26.090 / TS 26.190, ITU-T G.722.2).
//!
//! # Status
//!
//! **Both decoders work and are bit-exact against the normative references** —
//! AMR-WB against TS 26.173 at all nine rates, AMR-NB against TS 26.073 at all
//! eight. Encoding still returns [`CodecError::FeatureNotEnabled`] — loudly,
//! naming the phase that will supply it, rather than returning silence.
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
pub mod mode;
pub mod nb;
pub mod payload;
/// Where the Apache-2.0 oracles agree with the normative references.
#[cfg(test)]
mod qualification;
pub mod rate;
pub mod sdp;
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
/// # Not yet implemented
///
/// Encoding fails with [`CodecError::FeatureNotEnabled`] naming the phase that
/// will supply it, rather than returning silence or garbage — a codec that
/// quietly produces wrong audio is far harder to diagnose than one that
/// refuses. So do comfort noise, gaps and lost frames, each naming the piece it
/// needs.
pub struct AmrCodec {
    variant: AmrVariant,
    mode_set: AmrModeSet,
    current_mode: AmrMode,
    octet_align: bool,
    dtx: bool,
    /// The decoder for this instance's variant.
    ///
    /// Boxed because each carries several kilobytes of filter history and
    /// `AmrCodec` is otherwise a handful of words that callers move freely.
    decoder: Decoder,
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

        Ok(Self {
            variant,
            mode_set,
            current_mode,
            octet_align: params.requires_octet_align(),
            dtx: params.dtx,
            decoder: Decoder::for_variant(variant),
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
                let state = if quality_ok {
                    nb::decoder::FrameState::Good
                } else {
                    nb::decoder::FrameState::Bad
                };
                let params = nb::bitstream::parse(mode.index(), data).ok_or_else(short)?;
                Ok(decoder
                    .decode_parameters(mode.index(), &params, state)
                    .to_vec())
            }
            #[cfg(feature = "amr-wb")]
            Decoder::WideBand(decoder) => {
                if !quality_ok {
                    // Narrowband conceals; wideband does not reach its
                    // concealment from the frame path yet, and decoding
                    // damaged bits as if they were good is the loud artefact
                    // concealment exists to prevent.
                    return Err(CodecError::feature_not_enabled(
                        "AMR-WB error concealment for damaged frames is not implemented yet \
                         (see codec-core/docs/AMR_IMPLEMENTATION_PLAN.md)",
                    ));
                }
                decoder.decode(mode, data).map(Vec::from).ok_or_else(short)
            }
            Decoder::Absent => Err(CodecError::feature_not_enabled(format!(
                "{} decoding needs its cargo feature enabled",
                self.variant
            ))),
        }
    }

    /// The variant this instance codes.
    #[must_use]
    pub const fn variant(&self) -> AmrVariant {
        self.variant
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

    /// Error returned by every path that needs the unimplemented DSP kernel.
    fn kernel_unimplemented(&self, operation: &str) -> CodecError {
        let phase = match (self.variant, operation) {
            (AmrVariant::WideBand, "decode") => "phase 4",
            (AmrVariant::WideBand, _) => "phase 5",
            (AmrVariant::NarrowBand, "decode") => "phase 6",
            (AmrVariant::NarrowBand, _) => "phase 7",
        };
        CodecError::feature_not_enabled(format!(
            "{} {operation} is not implemented yet (planned for {phase}; \
             see codec-core/docs/AMR_IMPLEMENTATION_PLAN.md)",
            self.variant
        ))
    }
}

impl AudioCodec for AmrCodec {
    fn encode(&mut self, _samples: &[i16]) -> Result<Vec<u8>> {
        Err(self.kernel_unimplemented("encode"))
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

    fn set_mode(&mut self, mode: u8) -> Result<()> {
        let requested = AmrMode::new(self.variant, mode)?;
        if !self.mode_set.contains(requested) {
            return Err(CodecError::invalid_config(format!(
                "{} mode {mode} is outside the negotiated mode-set ({})",
                self.variant,
                self.mode_set.to_sdp_value()
            )));
        }
        self.current_mode = requested;
        Ok(())
    }

    fn encode_frame(&mut self, _samples: &[i16]) -> Result<CodedFrame> {
        Err(self.kernel_unimplemented("encode"))
    }

    fn decode_frame(&mut self, frame: &CodedFrame) -> Result<Vec<i16>> {
        match frame.kind {
            FrameKind::Speech => {
                let mode = AmrMode::new(self.variant, frame.mode)?;
                self.decode_frame_bits(mode, &frame.data, frame.quality_ok)
            }
            FrameKind::ComfortNoise | FrameKind::NoData | FrameKind::Lost => {
                Err(CodecError::feature_not_enabled(format!(
                    "AMR {:?} frames need DTX/comfort-noise and concealment, which are \
                     not implemented yet (see codec-core/docs/AMR_IMPLEMENTATION_PLAN.md)",
                    frame.kind
                )))
            }
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
    fn unimplemented_operations_fail_loudly_rather_than_returning_silence() {
        let mut wb = AmrCodec::new(&CodecConfig::amr_wb()).unwrap();

        let err = wb.encode(&vec![0i16; 320]).unwrap_err();
        assert!(matches!(err, CodecError::FeatureNotEnabled { .. }));
        // The message should say what is missing and where to read about it.
        let text = err.to_string();
        assert!(text.contains("AMR-WB"), "{text}");
        assert!(text.contains("not implemented"), "{text}");

        assert!(wb.encode_frame(&vec![0i16; 320]).is_err());
        // Comfort noise, gaps and losses all need machinery that does not
        // exist yet, and each is a different kind of missing.
        assert!(wb.decode_frame(&CodedFrame::no_data()).is_err());
        assert!(wb.decode_frame(&CodedFrame::lost()).is_err());
        assert!(wb.decode_frame(&CodedFrame::comfort_noise(vec![0; 5])).is_err());

        let mut nb = AmrCodec::new(&CodecConfig::amr_nb()).unwrap();
        assert!(nb.encode(&vec![0i16; 160]).is_err());
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
