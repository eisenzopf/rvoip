//! AMR narrowband, 3GPP TS 26.090 (prose) and TS 26.073 (the normative
//! fixed-point reference).
//!
//! 8 kHz, 160 samples per 20 ms frame, four 40-sample subframes, LP order 10,
//! eight rates from 4.75 to 12.2 kbit/s.
//!
//! # Relationship to the wideband implementation
//!
//! The two codecs are close relatives and it is tempting to share more than is
//! safe. They differ in ways that are invisible at a glance and produce audio
//! that sounds right while being wrong:
//!
//! - Narrowband uses **LSP/LSF**, wideband **ISP/ISF**. These are different
//!   representations, not the same one at a different order.
//! - Narrowband has a **decoder post-filter**; wideband has none.
//! - Each narrowband rate has its **own algebraic codebook**, where wideband
//!   has one family parameterised by width.
//! - `packed_size` in the two references uses different conventions: the
//!   narrowband table includes the `ToC` byte, the wideband one does not.
//!
//! Anything genuinely shared lives in [`crate::fixed_point`] or in the
//! variant-agnostic modules one level up ([`super::payload`],
//! [`super::storage`], [`super::mode`]).

pub mod bitstream;
pub mod cn;
pub mod codebook;
pub mod conceal;
pub mod decoder;
pub mod decoder_tables;
pub mod detect;
pub mod dtx;
pub mod enc;
pub mod gain;
pub mod homing;
pub mod lag;
pub mod lsp;
pub mod math;
pub mod postfilter;
pub mod synthesis;
pub mod tables;
#[cfg(test)]
pub mod vectors;

/// Frame size in samples, 20 ms at 8 kHz.
pub const L_FRAME: usize = 160;

/// Subframe size in samples; four to a frame.
pub const L_SUBFR: usize = 40;

/// Minimum pitch lag for every rate except 12.2 kbit/s.
pub const PIT_MIN: i16 = 20;

/// Minimum pitch lag at 12.2 kbit/s, which resolves the lag more finely and so
/// can afford to reach a shorter period.
pub const PIT_MIN_MR122: i16 = 18;

/// Maximum pitch lag, all rates.
pub const PIT_MAX: i16 = 143;

/// Length of the adaptive codebook's interpolation filter, one side plus one.
pub const L_INTERPOL: usize = 11;

/// Maximum pitch-sharpening factor, 0.7947 in Q14.
pub const SHARPMAX: i16 = 13017;

/// Tilt-compensation factor for the post-filter, 0.8 in Q15.
pub const MU: i16 = 26214;

/// Gain-smoothing factor for the post-filter's AGC, 0.9 in Q15.
pub const AGC_FAC: i16 = 29491;
