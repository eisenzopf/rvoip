#![allow(clippy::needless_range_loop)]
//! Provenance: Fixed codebook search/decode adapted from ITU G.729 ACELP algebraic codebook routines.
//! Q-format: Correlation buffers and pulse gains use Q13/Q15 with 32-bit fixed-point accumulators.

use crate::constants::L_SUBFR;
use crate::dsp::arith::{add, mult, negate, sub};
use crate::dsp::shift::{shl, shr};
use crate::dsp::types::{DspContext, Word16};
use crate::fixed_cb::d4i_state::D4iWorkspace;

pub(crate) fn finalize_d4i(
    ctx: &mut DspContext,
    h: &[i16; L_SUBFR],
    cod: &mut [i16; L_SUBFR],
    y: &mut [i16; L_SUBFR],
    sign: &mut i16,
    ws: &D4iWorkspace,
) -> i16 {
    let i0s = ws.sign_dn[ws.ip0 as usize];
    let i1s = ws.sign_dn[ws.ip1 as usize];
    let i2s = ws.sign_dn[ws.ip2 as usize];
    let i3s = ws.sign_dn[ws.ip3 as usize];

    cod.fill(0);
    cod[ws.ip0 as usize] = shr(ctx, Word16(i0s), 2).0;
    cod[ws.ip1 as usize] = shr(ctx, Word16(i1s), 2).0;
    cod[ws.ip2 as usize] = shr(ctx, Word16(i2s), 2).0;
    cod[ws.ip3 as usize] = shr(ctx, Word16(i3s), 2).0;

    y[..ws.ip0 as usize].fill(0);
    if i0s > 0 {
        for i in ws.ip0 as usize..L_SUBFR {
            y[i] = h[i - ws.ip0 as usize];
        }
    } else {
        for i in ws.ip0 as usize..L_SUBFR {
            y[i] = negate(ctx, Word16(h[i - ws.ip0 as usize])).0;
        }
    }
    if i1s > 0 {
        for i in ws.ip1 as usize..L_SUBFR {
            y[i] = add(ctx, Word16(y[i]), Word16(h[i - ws.ip1 as usize])).0;
        }
    } else {
        for i in ws.ip1 as usize..L_SUBFR {
            y[i] = sub(ctx, Word16(y[i]), Word16(h[i - ws.ip1 as usize])).0;
        }
    }
    if i2s > 0 {
        for i in ws.ip2 as usize..L_SUBFR {
            y[i] = add(ctx, Word16(y[i]), Word16(h[i - ws.ip2 as usize])).0;
        }
    } else {
        for i in ws.ip2 as usize..L_SUBFR {
            y[i] = sub(ctx, Word16(y[i]), Word16(h[i - ws.ip2 as usize])).0;
        }
    }
    if i3s > 0 {
        for i in ws.ip3 as usize..L_SUBFR {
            y[i] = add(ctx, Word16(y[i]), Word16(h[i - ws.ip3 as usize])).0;
        }
    } else {
        for i in ws.ip3 as usize..L_SUBFR {
            y[i] = sub(ctx, Word16(y[i]), Word16(h[i - ws.ip3 as usize])).0;
        }
    }

    let mut sgn = 0i16;
    if i0s > 0 {
        sgn = add(ctx, Word16(sgn), Word16(1)).0;
    }
    if i1s > 0 {
        sgn = add(ctx, Word16(sgn), Word16(2)).0;
    }
    if i2s > 0 {
        sgn = add(ctx, Word16(sgn), Word16(4)).0;
    }
    if i3s > 0 {
        sgn = add(ctx, Word16(sgn), Word16(8)).0;
    }
    *sign = sgn;

    let ip0i = mult(ctx, Word16(ws.ip0), Word16(6554)).0;
    let ip1i = mult(ctx, Word16(ws.ip1), Word16(6554)).0;
    let ip2i = mult(ctx, Word16(ws.ip2), Word16(6554)).0;
    let mut i = mult(ctx, Word16(ws.ip3), Word16(6554)).0;
    let i5 = shl(ctx, Word16(i), 2);
    let mut j = add(ctx, Word16(i), i5).0;
    let j3 = add(ctx, Word16(j), Word16(3));
    j = sub(ctx, Word16(ws.ip3), j3).0;
    let i2 = shl(ctx, Word16(i), 1);
    let ip3i = add(ctx, i2, Word16(j)).0;

    let ip1s = shl(ctx, Word16(ip1i), 3);
    i = add(ctx, Word16(ip0i), ip1s).0;
    let ip2s = shl(ctx, Word16(ip2i), 6);
    i = add(ctx, Word16(i), ip2s).0;
    let ip3s = shl(ctx, Word16(ip3i), 9);
    add(ctx, Word16(i), ip3s).0
}
