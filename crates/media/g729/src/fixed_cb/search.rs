//! ACELP fixed codebook search.
//! Provenance: Fixed codebook search/decode adapted from ITU G.729 ACELP algebraic codebook routines.
//! Q-format: Correlation buffers and pulse gains use Q13/Q15 with 32-bit fixed-point accumulators.

use crate::constants::{DIM_RR, L_SUBFR};
use crate::dsp::arith::{add, mult};
use crate::dsp::shift::shl;
use crate::dsp::types::{DspContext, Word16};

#[allow(clippy::too_many_arguments)]
pub(crate) fn acelp_code_a(
    x: &[i16; L_SUBFR],
    h: &mut [i16; L_SUBFR],
    t0: i16,
    pitch_sharp: i16,
    code: &mut [i16; L_SUBFR],
    y: &mut [i16; L_SUBFR],
    sign: &mut i16,
) -> i16 {
    let mut ctx = DspContext::default();
    let sharp = shl(&mut ctx, Word16(pitch_sharp), 1).0;
    if t0 < L_SUBFR as i16 {
        for i in t0 as usize..L_SUBFR {
            let m = mult(&mut ctx, Word16(h[i - t0 as usize]), Word16(sharp));
            h[i] = add(&mut ctx, Word16(h[i]), m).0;
        }
    }

    let mut rr = [0i16; DIM_RR];
    crate::fixed_cb::correlation::correlate_impulse_response(h, &mut rr);
    let mut dn = [0i16; L_SUBFR];
    crate::fixed_cb::correlation::correlate_target_with_impulse(h, x, &mut dn);
    let index = crate::fixed_cb::d4i::d4i40_17_fast(&mut dn, &rr, h, code, y, sign);

    if t0 < L_SUBFR as i16 {
        for i in t0 as usize..L_SUBFR {
            let m = mult(&mut ctx, Word16(code[i - t0 as usize]), Word16(sharp));
            code[i] = add(&mut ctx, Word16(code[i]), m).0;
        }
    }

    index
}

/// Compatibility surface used by the encode pipeline.
#[inline(always)]
#[allow(clippy::too_many_arguments)]
pub(crate) fn search_acelp_codebook(
    xn: &[i16; L_SUBFR],
    h: &mut [i16; L_SUBFR],
    t0: i16,
    pitch_sharp: i16,
    code: &mut [i16; L_SUBFR],
    y: &mut [i16; L_SUBFR],
    sign: &mut i16,
) -> i16 {
    acelp_code_a(xn, h, t0, pitch_sharp, code, y, sign)
}
