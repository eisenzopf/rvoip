//! Provenance: LPC/LSP analysis derived from ITU G.729 LPC conversion and recursion routines.
//! Q-format: Correlations and predictor coefficients use Q12/Q13/Q15 fixed-point domains.

use crate::codecs::g729::impls::constants::M;
#[cfg(feature = "g729")]
use crate::codecs::g729::impls::constants::NP;
use crate::codecs::g729::impls::dsp::oper32::{l_extract, mpy_32};
use crate::codecs::g729::impls::dsp::types::Word16;
use crate::codecs::g729::impls::tables::annexa::{LAG_H, LAG_L};

pub(crate) fn lag_window_10(r_h: &mut [i16; M + 1], r_l: &mut [i16; M + 1]) {
    for i in 1..=M {
        let x = mpy_32(
            Word16(r_h[i]),
            Word16(r_l[i]),
            Word16(LAG_H[i - 1]),
            Word16(LAG_L[i - 1]),
        );
        let (hi, lo) = l_extract(x);
        r_h[i] = hi.0;
        r_l[i] = lo.0;
    }
}

#[cfg(feature = "g729")]
pub(crate) fn lag_window_np(r_h: &mut [i16; NP + 1], r_l: &mut [i16; NP + 1]) {
    for i in 1..=NP {
        let x = mpy_32(
            Word16(r_h[i]),
            Word16(r_l[i]),
            Word16(LAG_H[i - 1]),
            Word16(LAG_L[i - 1]),
        );
        let (hi, lo) = l_extract(x);
        r_h[i] = hi.0;
        r_l[i] = lo.0;
    }
}
