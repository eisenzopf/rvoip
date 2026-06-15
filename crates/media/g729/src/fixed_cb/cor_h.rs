#![allow(clippy::needless_range_loop)]
//! Provenance: Fixed codebook search/decode adapted from ITU G.729 ACELP algebraic codebook routines.
//! Q-format: Correlation buffers and pulse gains use Q13/Q15 with 32-bit fixed-point accumulators.

use crate::constants::{DIM_RR, L_SUBFR, NB_POS};
use crate::dsp::arith::{extract_h, sub};
use crate::dsp::arith32::l_mac;
use crate::dsp::shift::{norm_l, shl, shr};
use crate::dsp::types::{DspContext, Word16, Word32};
use crate::fixed_cb::search_indices::{RRI0I0, RRI1I1, RRI2I2, RRI3I3, RRI4I4};

fn scale_impulse(ctx: &mut DspContext, h_in: &[i16; L_SUBFR], h: &mut [i16; L_SUBFR]) {
    let mut cor = Word32(0);
    for &v in h_in {
        cor = l_mac(ctx, cor, Word16(v), Word16(v));
    }

    if sub(ctx, extract_h(cor), Word16(32000)).0 > 0 {
        for i in 0..L_SUBFR {
            h[i] = shr(ctx, Word16(h_in[i]), 1).0;
        }
    } else {
        let mut k = norm_l(cor);
        k = shr(ctx, Word16(k), 1).0;
        for i in 0..L_SUBFR {
            h[i] = shl(ctx, Word16(h_in[i]), k).0;
        }
    }
}

fn fill_diag(ctx: &mut DspContext, h: &[i16; L_SUBFR], rr: &mut [i16; DIM_RR]) {
    let mut p0 = (RRI0I0 + NB_POS - 1) as isize;
    let mut p1 = (RRI1I1 + NB_POS - 1) as isize;
    let mut p2 = (RRI2I2 + NB_POS - 1) as isize;
    let mut p3 = (RRI3I3 + NB_POS - 1) as isize;
    let mut p4 = (RRI4I4 + NB_POS - 1) as isize;

    let mut ptr_h1 = 0usize;
    let mut cor = Word32(0);
    for _ in 0..NB_POS {
        cor = l_mac(ctx, cor, Word16(h[ptr_h1]), Word16(h[ptr_h1]));
        ptr_h1 += 1;
        rr[p4 as usize] = extract_h(cor).0;
        p4 -= 1;

        cor = l_mac(ctx, cor, Word16(h[ptr_h1]), Word16(h[ptr_h1]));
        ptr_h1 += 1;
        rr[p3 as usize] = extract_h(cor).0;
        p3 -= 1;

        cor = l_mac(ctx, cor, Word16(h[ptr_h1]), Word16(h[ptr_h1]));
        ptr_h1 += 1;
        rr[p2 as usize] = extract_h(cor).0;
        p2 -= 1;

        cor = l_mac(ctx, cor, Word16(h[ptr_h1]), Word16(h[ptr_h1]));
        ptr_h1 += 1;
        rr[p1 as usize] = extract_h(cor).0;
        p1 -= 1;

        cor = l_mac(ctx, cor, Word16(h[ptr_h1]), Word16(h[ptr_h1]));
        ptr_h1 += 1;
        rr[p0 as usize] = extract_h(cor).0;
        p0 -= 1;
    }
}

pub(crate) fn cor_h(h_in: &[i16; L_SUBFR], rr: &mut [i16; DIM_RR]) {
    let mut ctx = DspContext::default();
    let mut h = [0i16; L_SUBFR];
    scale_impulse(&mut ctx, h_in, &mut h);
    fill_diag(&mut ctx, &h, rr);
    crate::fixed_cb::cor_h_cross_a::fill_cross_1(&mut ctx, &h, rr);
    crate::fixed_cb::cor_h_cross_a::fill_cross_2(&mut ctx, &h, rr);
    crate::fixed_cb::cor_h_cross_b::fill_cross_3(&mut ctx, &h, rr);
    crate::fixed_cb::cor_h_cross_b::fill_cross_4(&mut ctx, &h, rr);
}
