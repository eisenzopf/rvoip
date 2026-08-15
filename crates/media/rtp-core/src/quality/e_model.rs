//! Canonical reduced-input E-model implementation for this crate.
//!
//! The rating equation and random-loss equipment impairment follow ITU-T
//! G.107 sections 7.1 and 7.5, and R-to-MOS conversion follows Annex B. The
//! piecewise-linear delay term is a transport-monitoring approximation of the
//! full G.107 section 7.4 calculation; it is not the full planning model. Delay
//! is one-way mouth-to-ear time as defined by ITU-T G.114.
//!
//! Codec impairment parameters come from ITU-T G.113 Appendix I where an
//! exact codec, packetization, and PLC combination is available. Packet loss
//! is modeled as random loss; burst loss will degrade speech more than these
//! scores predict. The advantage factor `A` is fixed at zero, which is
//! appropriate for fixed telephony and conservative for mobile use.
//!
//! References:
//! - ITU-T G.107: <https://www.itu.int/rec/T-REC-G.107>
//! - ITU-T G.113: <https://www.itu.int/rec/T-REC-G.113>
//! - ITU-T G.114: <https://www.itu.int/rec/T-REC-G.114>

/// Maximum transmission rating without an advantage factor.
pub const R_FACTOR_MAX: f32 = 93.2;
const MOS_MIN: f32 = 1.0;
const MOS_MAX: f32 = 4.5;
const MOS_FLOOR_R_FACTOR: f32 = 6.52;
const DELAY_KNEE_MS: f32 = 177.3;

/// Codec impairment parameters from ITU-T G.113 Appendix I.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct CodecImpairment {
    /// Equipment impairment factor at zero packet loss.
    pub ie: f32,
    /// Packet loss robustness factor.
    pub bpl: f32,
}

/// G.711 PCMU/PCMA with 10 ms packets and G.711 Appendix I PLC.
pub const G711: CodecImpairment = CodecImpairment { ie: 0.0, bpl: 25.1 };
/// Approximate G.722 parameters for the narrowband compatibility model.
///
/// G.722 is a wideband codec and should use ITU-T G.107.1. ITU-T G.113 does
/// not publish this narrowband `(Ie, Bpl)` pair; retain it only for callers
/// that require the reduced model.
pub const G722: CodecImpairment = CodecImpairment {
    ie: 13.0,
    bpl: 21.0,
};
/// G.729 Annex A with Annex B VAD, 20 ms packets, and native PLC.
pub const G729: CodecImpairment = CodecImpairment {
    ie: 11.0,
    bpl: 19.0,
};
/// G.723.1 at 6.3 kbit/s with VAD, 30 ms packets, and native PLC.
pub const G723_1_63K: CodecImpairment = CodecImpairment {
    ie: 15.0,
    bpl: 16.1,
};
/// Approximate Opus impairment parameters at 24 kbit/s.
///
/// ITU-T G.113 Appendix I does not define Opus parameters.
pub const OPUS_24K: CodecImpairment = CodecImpairment { ie: 7.0, bpl: 24.0 };

/// Inputs to one E-model evaluation, all in their natural units.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QualityInputs {
    /// One-way delay in milliseconds, not round trip.
    pub one_way_delay_ms: f32,
    /// Effective packet loss percentage, including jitter-buffer discards.
    pub loss_percent: f32,
    /// Equipment impairment parameters for the encoded audio.
    pub codec: CodecImpairment,
}

/// Listening and conversational scores from one E-model evaluation.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct QualityScores {
    /// Transmission rating factor, from 0 to [`R_FACTOR_MAX`].
    pub r_factor: f32,
    /// Listening quality with delay excluded, from 1.0 to 4.5.
    pub mos_lq: f32,
    /// Conversational quality with delay included, from 1.0 to 4.5.
    pub mos_cq: f32,
}

/// Evaluate voice quality using ITU-T G.107.
///
/// # Arguments
///
/// * `inputs` - Codec, effective loss, and one-way delay for the evaluation.
pub fn evaluate(inputs: QualityInputs) -> QualityScores {
    let ie_eff = equipment_impairment(inputs.loss_percent, inputs.codec);
    let listening_r = (R_FACTOR_MAX - ie_eff).clamp(0.0, R_FACTOR_MAX);
    let delay = sanitize_non_negative(inputs.one_way_delay_ms);
    let conversational_r = (listening_r - delay_impairment(delay)).clamp(0.0, R_FACTOR_MAX);

    QualityScores {
        r_factor: conversational_r,
        mos_lq: r_factor_to_mos(listening_r),
        mos_cq: r_factor_to_mos(conversational_r),
    }
}

/// Convert an ITU-T G.107 transmission rating factor to MOS.
///
/// # Arguments
///
/// * `r_factor` - Transmission rating factor to convert.
pub fn r_factor_to_mos(r_factor: f32) -> f32 {
    let r = sanitize_non_negative(r_factor).clamp(0.0, R_FACTOR_MAX);
    if r < MOS_FLOOR_R_FACTOR {
        return MOS_MIN;
    }

    let mos = 1.0 + 0.035 * r + 0.000_007 * r * (r - 60.0) * (100.0 - r);
    mos.clamp(MOS_MIN, MOS_MAX)
}

fn delay_impairment(delay_ms: f32) -> f32 {
    let beyond_knee = (delay_ms - DELAY_KNEE_MS).max(0.0);

    0.024 * delay_ms + 0.11 * beyond_knee
}

fn equipment_impairment(loss_percent: f32, codec: CodecImpairment) -> f32 {
    let loss = sanitize_non_negative(loss_percent).clamp(0.0, 100.0);
    let ie = sanitize_non_negative(codec.ie).clamp(0.0, 95.0);
    let bpl = sanitize_non_negative(codec.bpl).max(f32::EPSILON);

    ie + (95.0 - ie) * loss / (loss + bpl)
}

fn sanitize_non_negative(value: f32) -> f32 {
    if value.is_nan() {
        return 0.0;
    }

    value.max(0.0)
}
