#![allow(clippy::needless_range_loop)]
//! Provenance: Fixed codebook search/decode adapted from ITU G.729 ACELP algebraic codebook routines.
//! Q-format: Correlation buffers and pulse gains use Q13/Q15 with 32-bit fixed-point accumulators.

use crate::constants::{DIM_RR, L_SUBFR, MSIZE, NB_POS, STEP};
use crate::dsp::arith::extract_h;
use crate::dsp::arith32::l_mac;
use crate::dsp::types::{DspContext, Word16, Word32};
use crate::fixed_cb::search_indices::{
    RRI0I1, RRI0I2, RRI0I3, RRI0I4, RRI1I2, RRI1I3, RRI1I4, RRI2I3, RRI2I4,
};

pub(crate) fn fill_cross_3(ctx: &mut DspContext, h: &[i16; L_SUBFR], rr: &mut [i16; DIM_RR]) {
    let ptr_hd = 0usize;
    let mut ptr_hf = ptr_hd + 3;
    let mut l_fin_sup = (MSIZE - 1) as isize;
    let mut l_fin_inf = l_fin_sup - 1;
    let ldec = (NB_POS + 1) as isize;

    for k in 0..NB_POS {
        let mut p4 = (RRI1I4 as isize) + l_fin_sup;
        let mut p3 = (RRI0I3 as isize) + l_fin_sup;
        let mut p2 = (RRI2I4 as isize) + l_fin_inf;
        let mut p1 = (RRI1I3 as isize) + l_fin_inf;
        let mut p0 = (RRI0I2 as isize) + l_fin_inf;

        let mut ptr_h1 = ptr_hd;
        let mut ptr_h2 = ptr_hf;
        let mut cor = Word32(0);
        for _ in (k + 1)..NB_POS {
            cor = l_mac(ctx, cor, Word16(h[ptr_h1]), Word16(h[ptr_h2]));
            ptr_h1 += 1;
            ptr_h2 += 1;
            rr[p4 as usize] = extract_h(cor).0;

            cor = l_mac(ctx, cor, Word16(h[ptr_h1]), Word16(h[ptr_h2]));
            ptr_h1 += 1;
            ptr_h2 += 1;
            rr[p3 as usize] = extract_h(cor).0;

            cor = l_mac(ctx, cor, Word16(h[ptr_h1]), Word16(h[ptr_h2]));
            ptr_h1 += 1;
            ptr_h2 += 1;
            rr[p2 as usize] = extract_h(cor).0;

            cor = l_mac(ctx, cor, Word16(h[ptr_h1]), Word16(h[ptr_h2]));
            ptr_h1 += 1;
            ptr_h2 += 1;
            rr[p1 as usize] = extract_h(cor).0;

            cor = l_mac(ctx, cor, Word16(h[ptr_h1]), Word16(h[ptr_h2]));
            ptr_h1 += 1;
            ptr_h2 += 1;
            rr[p0 as usize] = extract_h(cor).0;

            p4 -= ldec;
            p3 -= ldec;
            p2 -= ldec;
            p1 -= ldec;
            p0 -= ldec;
        }

        cor = l_mac(ctx, cor, Word16(h[ptr_h1]), Word16(h[ptr_h2]));
        ptr_h1 += 1;
        ptr_h2 += 1;
        rr[p4 as usize] = extract_h(cor).0;

        cor = l_mac(ctx, cor, Word16(h[ptr_h1]), Word16(h[ptr_h2]));
        rr[p3 as usize] = extract_h(cor).0;

        l_fin_sup -= NB_POS as isize;
        l_fin_inf -= 1;
        ptr_hf += STEP;
    }
}

pub(crate) fn fill_cross_4(ctx: &mut DspContext, h: &[i16; L_SUBFR], rr: &mut [i16; DIM_RR]) {
    let ptr_hd = 0usize;
    let mut ptr_hf = ptr_hd + 4;
    let mut l_fin_sup = (MSIZE - 1) as isize;
    let mut l_fin_inf = l_fin_sup - 1;
    let ldec = (NB_POS + 1) as isize;

    for k in 0..NB_POS {
        let mut p3 = (RRI0I4 as isize) + l_fin_sup;
        let mut p2 = (RRI2I3 as isize) + l_fin_inf;
        let mut p1 = (RRI1I2 as isize) + l_fin_inf;
        let mut p0 = (RRI0I1 as isize) + l_fin_inf;

        let mut ptr_h1 = ptr_hd;
        let mut ptr_h2 = ptr_hf;
        let mut cor = Word32(0);
        for _ in (k + 1)..NB_POS {
            cor = l_mac(ctx, cor, Word16(h[ptr_h1]), Word16(h[ptr_h2]));
            ptr_h1 += 1;
            ptr_h2 += 1;
            rr[p3 as usize] = extract_h(cor).0;

            cor = l_mac(ctx, cor, Word16(h[ptr_h1]), Word16(h[ptr_h2]));
            ptr_h1 += 1;
            ptr_h2 += 1;
            cor = l_mac(ctx, cor, Word16(h[ptr_h1]), Word16(h[ptr_h2]));
            ptr_h1 += 1;
            ptr_h2 += 1;
            rr[p2 as usize] = extract_h(cor).0;

            cor = l_mac(ctx, cor, Word16(h[ptr_h1]), Word16(h[ptr_h2]));
            ptr_h1 += 1;
            ptr_h2 += 1;
            rr[p1 as usize] = extract_h(cor).0;

            cor = l_mac(ctx, cor, Word16(h[ptr_h1]), Word16(h[ptr_h2]));
            ptr_h1 += 1;
            ptr_h2 += 1;
            rr[p0 as usize] = extract_h(cor).0;

            p3 -= ldec;
            p2 -= ldec;
            p1 -= ldec;
            p0 -= ldec;
        }

        cor = l_mac(ctx, cor, Word16(h[ptr_h1]), Word16(h[ptr_h2]));
        rr[p3 as usize] = extract_h(cor).0;

        l_fin_sup -= NB_POS as isize;
        l_fin_inf -= 1;
        ptr_hf += STEP;
    }
}
