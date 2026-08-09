//! AMR-NB and AMR-WB codecs (3GPP TS 26.090 / TS 26.190, ITU-T G.722.2).
//!
//! # Status
//!
//! **Phase 0 — types only. There is no encoder or decoder yet.** Constructing a
//! codec succeeds and reports its configuration, but `encode`/`decode` return
//! [`CodecError::FeatureNotEnabled`]. This lets the payload format, SDP
//! negotiation and pipeline plumbing be built and tested against a real type
//! before the DSP kernel exists.
//!
//! See `docs/AMR_IMPLEMENTATION_PLAN.md` for the phased plan and
//! `docs/AMR_IMPLEMENTATION_STATUS.md` for current progress.
//!
//! # Structure
//!
//! [`mode`] carries the codec modes, frame types and mode-set negotiation
//! logic — the parts that are pure specification data and are needed by the RTP
//! payload format and SDP layers well before any signal processing.

use crate::error::{CodecError, Result};
use crate::types::{
    AudioCodec, CodecConfig, CodecInfo, CodecType, CodedFrame, VariableRateCodec,
};

pub mod mode;

pub use mode::{AmrFrameType, AmrMode, AmrModeSet, AmrVariant};

/// AMR-NB / AMR-WB codec.
///
/// # Not yet implemented
///
/// The speech coding kernel is unimplemented (see the module docs). Encode and
/// decode calls fail with [`CodecError::FeatureNotEnabled`] naming the phase
/// that will supply them, rather than returning silence or garbage — a codec
/// that quietly produces wrong audio is far harder to diagnose than one that
/// refuses.
pub struct AmrCodec {
    variant: AmrVariant,
    mode_set: AmrModeSet,
    current_mode: AmrMode,
    octet_align: bool,
    dtx: bool,
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
        })
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

    fn decode(&mut self, _data: &[u8]) -> Result<Vec<i16>> {
        Err(self.kernel_unimplemented("decode"))
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
        // No kernel state to clear yet. Mode selection deliberately survives a
        // reset: it is negotiated state, not stream state.
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

    fn decode_frame(&mut self, _frame: &CodedFrame) -> Result<Vec<i16>> {
        Err(self.kernel_unimplemented("decode"))
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
    fn kernel_operations_fail_loudly_rather_than_returning_silence() {
        let mut wb = AmrCodec::new(&CodecConfig::amr_wb()).unwrap();

        let err = wb.encode(&vec![0i16; 320]).unwrap_err();
        assert!(matches!(err, CodecError::FeatureNotEnabled { .. }));
        // The message should say what is missing and where to read about it.
        let text = err.to_string();
        assert!(text.contains("AMR-WB"), "{text}");
        assert!(text.contains("not implemented"), "{text}");

        assert!(wb.decode(&[0u8; 60]).is_err());
        assert!(wb.encode_frame(&vec![0i16; 320]).is_err());
        assert!(wb.decode_frame(&CodedFrame::no_data()).is_err());
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
