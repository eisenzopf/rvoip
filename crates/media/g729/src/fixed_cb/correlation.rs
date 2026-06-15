#![allow(clippy::needless_range_loop)]
//! Provenance: Fixed codebook search/decode adapted from ITU G.729 ACELP algebraic codebook routines.
//! Q-format: Correlation buffers and pulse gains use Q13/Q15 with 32-bit fixed-point accumulators.

use crate::constants::{DIM_RR, L_SUBFR};
use crate::dsp::arith::{add, extract_l, negate, round, sub};
use crate::dsp::arith32::{l_abs, l_mac, l_sub};
use crate::dsp::shift::{l_shl, l_shr, norm_l, shr};
use crate::dsp::types::{DspContext, Word16, Word32};

pub(crate) fn correlate_target_with_impulse(
    h: &[i16; L_SUBFR],
    x: &[i16; L_SUBFR],
    d: &mut [i16; L_SUBFR],
) {
    let mut ctx = DspContext::default();
    let mut max = Word32(0);
    let mut y32 = [Word32(0); L_SUBFR];

    for i in 0..L_SUBFR {
        let mut s = Word32(0);
        for j in i..L_SUBFR {
            s = l_mac(&mut ctx, s, Word16(x[j]), Word16(h[j - i]));
        }
        y32[i] = s;
        let a = l_abs(&mut ctx, s);
        if l_sub(&mut ctx, a, max).0 > 0 {
            max = a;
        }
    }

    let mut j = norm_l(max);
    if j > 16 {
        j = 16;
    }
    j = 18 - j;

    for i in 0..L_SUBFR {
        d[i] = extract_l(l_shr(&mut ctx, y32[i], j)).0;
    }
}

pub(crate) fn correlate_impulse_response(h: &[i16; L_SUBFR], rr: &mut [i16; DIM_RR]) {
    crate::fixed_cb::cor_h::cor_h(h, rr);
}

pub(crate) fn corr_xy2(
    xn: &[i16; L_SUBFR],
    y1: &[i16; L_SUBFR],
    y2: &[i16; L_SUBFR],
    g_coeff: &mut [i16; 5],
    exp_g_coeff: &mut [i16; 5],
) {
    let mut ctx = DspContext::default();
    let mut scaled_y2 = [0i16; L_SUBFR];
    for i in 0..L_SUBFR {
        scaled_y2[i] = shr(&mut ctx, Word16(y2[i]), 3).0;
    }

    let mut l_acc = Word32(1);
    for i in 0..L_SUBFR {
        l_acc = l_mac(&mut ctx, l_acc, Word16(scaled_y2[i]), Word16(scaled_y2[i]));
    }
    let exp = norm_l(l_acc);
    let t = l_shl(&mut ctx, l_acc, exp);
    let y2y2 = round(&mut ctx, t).0;
    let exp_y2y2 = add(&mut ctx, Word16(exp), Word16(19 - 16)).0;
    g_coeff[2] = y2y2;
    exp_g_coeff[2] = exp_y2y2;

    l_acc = Word32(1);
    for i in 0..L_SUBFR {
        l_acc = l_mac(&mut ctx, l_acc, Word16(xn[i]), Word16(scaled_y2[i]));
    }
    let exp = norm_l(l_acc);
    let t = l_shl(&mut ctx, l_acc, exp);
    let xny2 = round(&mut ctx, t).0;
    let exp_xny2 = add(&mut ctx, Word16(exp), Word16(10 - 16)).0;
    g_coeff[3] = negate(&mut ctx, Word16(xny2)).0;
    exp_g_coeff[3] = sub(&mut ctx, Word16(exp_xny2), Word16(1)).0;

    l_acc = Word32(1);
    for i in 0..L_SUBFR {
        l_acc = l_mac(&mut ctx, l_acc, Word16(y1[i]), Word16(scaled_y2[i]));
    }
    let exp = norm_l(l_acc);
    let t = l_shl(&mut ctx, l_acc, exp);
    let y1y2 = round(&mut ctx, t).0;
    let exp_y1y2 = add(&mut ctx, Word16(exp), Word16(10 - 16)).0;
    g_coeff[4] = y1y2;
    exp_g_coeff[4] = sub(&mut ctx, Word16(exp_y1y2), Word16(1)).0;
}
