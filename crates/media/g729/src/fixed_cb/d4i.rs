//! Provenance: Fixed codebook search/decode adapted from ITU G.729 ACELP algebraic codebook routines.
//! Q-format: Correlation buffers and pulse gains use Q13/Q15 with 32-bit fixed-point accumulators.

use crate::constants::{DIM_RR, L_SUBFR};
use crate::dsp::arith32::{l_msu, l_mult};
use crate::dsp::types::{DspContext, Word16};
use crate::fixed_cb::d4i_state::{Candidate, D4iWorkspace};
use crate::fixed_cb::search_indices::{RRI0I4, RRI1I4, RRI2I4, RRI4I4};

fn apply_candidate(ctx: &mut DspContext, ws: &mut D4iWorkspace, cand: Candidate) {
    let l_sq = l_mult(ctx, Word16(ws.alpk), Word16(cand.sq));
    let s = l_msu(ctx, l_sq, Word16(ws.psk), Word16(cand.alp));
    if s.0 > 0 {
        ws.psk = cand.sq;
        ws.alpk = cand.alp;
        ws.ip0 = cand.ip0;
        ws.ip1 = cand.ip1;
        ws.ip2 = cand.ip2;
        ws.ip3 = cand.ip3;
    }
}

pub(crate) fn d4i40_17_fast(
    dn: &mut [i16; L_SUBFR],
    rr: &[i16; DIM_RR],
    h: &[i16; L_SUBFR],
    cod: &mut [i16; L_SUBFR],
    y: &mut [i16; L_SUBFR],
    sign: &mut i16,
) -> i16 {
    let mut ctx = DspContext::default();
    let mut ws = D4iWorkspace::default();

    crate::fixed_cb::d4i_prepare::prepare_d4i(&mut ctx, dn, rr, &mut ws);

    for track in 3..5 {
        let cand_a = crate::fixed_cb::d4i_variant_a::search_variant_a(&mut ctx, track, dn, &mut ws);
        apply_candidate(&mut ctx, &mut ws, cand_a);

        let cand_b = crate::fixed_cb::d4i_variant_b::search_variant_b(&mut ctx, track, dn, &mut ws);
        apply_candidate(&mut ctx, &mut ws, cand_b);

        ws.ptr_rri0i3_i4 = RRI0I4;
        ws.ptr_rri1i3_i4 = RRI1I4;
        ws.ptr_rri2i3_i4 = RRI2I4;
        ws.ptr_rri3i3_i4 = RRI4I4;
    }

    crate::fixed_cb::d4i_finalize::finalize_d4i(&mut ctx, h, cod, y, sign, &ws)
}
