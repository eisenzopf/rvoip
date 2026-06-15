#![allow(dead_code)]
//! Provenance: Post-filter stages adapted from ITU G.729 Annex A formant/pitch/AGC post-processing.
//! Q-format: Post-filter coefficients and synthesis states use Q12/Q14/Q15 fixed-point paths.

use crate::constants::MP1;

/// Compatibility bridge to weighted LPC formant filtering coefficients.
#[inline(always)]
pub(crate) fn formant_weight(a: &[i16; MP1], gamma: i16, ap: &mut [i16; MP1]) {
    crate::lp::weight::weight_az_decode(a, gamma, ap);
}
