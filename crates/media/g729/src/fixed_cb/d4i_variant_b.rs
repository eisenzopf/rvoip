#![allow(clippy::needless_range_loop)]
#![allow(clippy::explicit_counter_loop)]
//! Provenance: Fixed codebook search/decode adapted from ITU G.729 ACELP algebraic codebook routines.
//! Q-format: Correlation buffers and pulse gains use Q13/Q15 with 32-bit fixed-point accumulators.

use crate::constants::{L_SUBFR, NB_POS, STEP};
use crate::dsp::arith::{add, mult, round, sub};
use crate::dsp::arith32::{l_mac, l_msu, l_mult};
use crate::dsp::shift::shl;
use crate::dsp::types::{DspContext, Word16};
use crate::fixed_cb::d4i_state::{
    Candidate, D4iWorkspace, ONE_EIGHTH, ONE_FOURTH, ONE_HALF, ONE_SIXTEENTH,
};
use crate::fixed_cb::search_indices::{RRI0I0, RRI0I1, RRI0I2, RRI1I1, RRI1I2, RRI2I2};

pub(crate) fn search_variant_b(
    ctx: &mut DspContext,
    track: usize,
    dn: &[i16; L_SUBFR],
    ws: &mut D4iWorkspace,
) -> Candidate {
    let mut sq = -1i16;
    let mut alp = 1i16;
    let mut ix = 0i16;
    let mut iy = 0i16;
    let mut ps = 0i16;
    let mut prev_i0 = -1i16;

    for _ in 0..2 {
        let mut max = -1i16;
        let mut i0 = track as i16;
        for j in (track..L_SUBFR).step_by(STEP) {
            if sub(ctx, Word16(dn[j]), Word16(max)).0 > 0
                && sub(ctx, Word16(prev_i0), Word16(j as i16)).0 != 0
            {
                max = dn[j];
                i0 = j as i16;
            }
        }
        prev_i0 = i0;

        let j = mult(ctx, Word16(i0), Word16(6554)).0 as usize;
        let mut p0 = ws.ptr_rri3i3_i4 + j;
        let ps1 = dn[i0 as usize];
        let alp1 = l_mult(ctx, Word16(ws.rr_mod[p0]), Word16(ONE_FOURTH));

        p0 = ws.ptr_rri0i3_i4 + j;
        let mut p1 = RRI0I0;
        for i1 in (0..L_SUBFR).step_by(STEP) {
            let ps2 = add(ctx, Word16(ps1), Word16(dn[i1])).0;
            let mut alp2 = l_mac(ctx, alp1, Word16(ws.rr_mod[p0]), Word16(ONE_HALF));
            p0 += NB_POS;
            alp2 = l_mac(ctx, alp2, Word16(ws.rr_mod[p1]), Word16(ONE_FOURTH));
            p1 += 1;
            let sq2 = mult(ctx, Word16(ps2), Word16(ps2)).0;
            let alp_16 = round(ctx, alp2).0;
            let l_sq = l_mult(ctx, Word16(alp), Word16(sq2));
            let s2 = l_msu(ctx, l_sq, Word16(sq), Word16(alp_16));
            if s2.0 > 0 {
                sq = sq2;
                ps = ps2;
                alp = alp_16;
                ix = i0;
                iy = i1 as i16;
            }
        }
    }

    let i0 = ix;
    let i1 = iy;
    let i1_div5 = mult(ctx, Word16(i1), Word16(6554));
    let i1_offset = shl(ctx, i1_div5, 3).0 as usize;

    let ps0 = ps;
    let alp0 = l_mult(ctx, Word16(alp), Word16(ONE_FOURTH));
    let mut sq = -1i16;
    let mut alp = 1i16;
    let mut ix = 0i16;
    let mut iy = 0i16;

    let mut p0 = ws.ptr_rri2i3_i4 + mult(ctx, Word16(i0), Word16(6554)).0 as usize;
    let mut p1 = RRI0I2 + i1_offset;
    let mut p2 = RRI2I2;
    for tv in &mut ws.tmp_vect {
        let mut s2 = l_mult(ctx, Word16(ws.rr_mod[p0]), Word16(ONE_FOURTH));
        p0 += NB_POS;
        s2 = l_mac(ctx, s2, Word16(ws.rr_mod[p1]), Word16(ONE_FOURTH));
        p1 += 1;
        s2 = l_mac(ctx, s2, Word16(ws.rr_mod[p2]), Word16(ONE_EIGHTH));
        p2 += 1;
        *tv = round(ctx, s2).0;
    }

    p0 = ws.ptr_rri1i3_i4 + mult(ctx, Word16(i0), Word16(6554)).0 as usize;
    p1 = RRI0I1 + i1_offset;
    p2 = RRI1I1;
    let mut p3 = RRI1I2;
    for i2 in (1..L_SUBFR).step_by(STEP) {
        let ps1 = add(ctx, Word16(ps0), Word16(dn[i2])).0;
        let mut alp1 = l_mac(ctx, alp0, Word16(ws.rr_mod[p0]), Word16(ONE_EIGHTH));
        p0 += NB_POS;
        alp1 = l_mac(ctx, alp1, Word16(ws.rr_mod[p1]), Word16(ONE_EIGHTH));
        p1 += 1;
        alp1 = l_mac(ctx, alp1, Word16(ws.rr_mod[p2]), Word16(ONE_SIXTEENTH));
        p2 += 1;

        let mut p4 = 0usize;
        let mut p3_loc = p3;
        for i3 in (2..L_SUBFR).step_by(STEP) {
            let ps2 = add(ctx, Word16(ps1), Word16(dn[i3])).0;
            let mut alp2 = l_mac(ctx, alp1, Word16(ws.rr_mod[p3_loc]), Word16(ONE_EIGHTH));
            p3_loc += 1;
            alp2 = l_mac(ctx, alp2, Word16(ws.tmp_vect[p4]), Word16(ONE_HALF));
            p4 += 1;
            let sq2 = mult(ctx, Word16(ps2), Word16(ps2)).0;
            let alp_16 = round(ctx, alp2).0;
            let l_sq = l_mult(ctx, Word16(alp), Word16(sq2));
            let s2 = l_msu(ctx, l_sq, Word16(sq), Word16(alp_16));
            if s2.0 > 0 {
                sq = sq2;
                alp = alp_16;
                ix = i2 as i16;
                iy = i3 as i16;
            }
        }
        p3 += NB_POS;
    }

    Candidate {
        sq,
        alp,
        ip0: i1,
        ip1: ix,
        ip2: iy,
        ip3: i0,
    }
}
