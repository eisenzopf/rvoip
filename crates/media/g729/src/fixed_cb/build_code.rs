#![allow(dead_code)]
//! Provenance: Fixed codebook search/decode adapted from ITU G.729 ACELP algebraic codebook routines.
//! Q-format: Correlation buffers and pulse gains use Q13/Q15 with 32-bit fixed-point accumulators.

//! Fixed codebook build/decode compatibility surface.

use crate::constants::L_SUBFR;
use crate::dsp::types::Word16;

/// Build a codevector from packed ACELP indices using the decoder primitive.
#[inline(always)]
pub(crate) fn build_codevector(sign: i16, index: i16) -> [i16; L_SUBFR] {
    let words = crate::fixed_cb::decod_acelp(Word16(sign), Word16(index));
    let mut out = [0i16; L_SUBFR];
    for i in 0..L_SUBFR {
        out[i] = words[i].0;
    }
    out
}
