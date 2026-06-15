#![allow(clippy::needless_range_loop)]
//! Provenance: Fixed codebook search/decode adapted from ITU G.729 ACELP algebraic codebook routines.
//! Q-format: Correlation buffers and pulse gains use Q13/Q15 with 32-bit fixed-point accumulators.

use crate::constants::{DIM_RR, L_SUBFR, STEP};
use crate::dsp::arith::{mult, negate};
use crate::dsp::types::{DspContext, Word16};
use crate::fixed_cb::d4i_state::D4iWorkspace;
use crate::fixed_cb::search_indices::{
    RRI0I1, RRI0I2, RRI0I3, RRI0I4, RRI1I2, RRI1I3, RRI1I4, RRI2I3, RRI2I4, RRI3I3,
};

pub(crate) fn prepare_d4i(
    ctx: &mut DspContext,
    dn: &mut [i16; L_SUBFR],
    rr: &[i16; DIM_RR],
    ws: &mut D4iWorkspace,
) {
    for i in 0..L_SUBFR {
        if dn[i] >= 0 {
            ws.sign_dn[i] = i16::MAX;
            ws.sign_dn_inv[i] = i16::MIN;
        } else {
            ws.sign_dn[i] = i16::MIN;
            ws.sign_dn_inv[i] = i16::MAX;
            dn[i] = negate(ctx, Word16(dn[i])).0;
        }
    }

    ws.rr_mod = *rr;

    let mut p0 = RRI0I1;
    let mut p1 = RRI0I2;
    let mut p2 = RRI0I3;
    let mut p3 = RRI0I4;
    for i0 in (0..L_SUBFR).step_by(STEP) {
        let psign = if ws.sign_dn[i0] < 0 {
            &ws.sign_dn_inv
        } else {
            &ws.sign_dn
        };
        for i1 in (1..L_SUBFR).step_by(STEP) {
            ws.rr_mod[p0] = mult(ctx, Word16(ws.rr_mod[p0]), Word16(psign[i1])).0;
            ws.rr_mod[p1] = mult(ctx, Word16(ws.rr_mod[p1]), Word16(psign[i1 + 1])).0;
            ws.rr_mod[p2] = mult(ctx, Word16(ws.rr_mod[p2]), Word16(psign[i1 + 2])).0;
            ws.rr_mod[p3] = mult(ctx, Word16(ws.rr_mod[p3]), Word16(psign[i1 + 3])).0;
            p0 += 1;
            p1 += 1;
            p2 += 1;
            p3 += 1;
        }
    }

    p0 = RRI1I2;
    p1 = RRI1I3;
    p2 = RRI1I4;
    for i1 in (1..L_SUBFR).step_by(STEP) {
        let psign = if ws.sign_dn[i1] < 0 {
            &ws.sign_dn_inv
        } else {
            &ws.sign_dn
        };
        for i2 in (2..L_SUBFR).step_by(STEP) {
            ws.rr_mod[p0] = mult(ctx, Word16(ws.rr_mod[p0]), Word16(psign[i2])).0;
            ws.rr_mod[p1] = mult(ctx, Word16(ws.rr_mod[p1]), Word16(psign[i2 + 1])).0;
            ws.rr_mod[p2] = mult(ctx, Word16(ws.rr_mod[p2]), Word16(psign[i2 + 2])).0;
            p0 += 1;
            p1 += 1;
            p2 += 1;
        }
    }

    p0 = RRI2I3;
    p1 = RRI2I4;
    for i2 in (2..L_SUBFR).step_by(STEP) {
        let psign = if ws.sign_dn[i2] < 0 {
            &ws.sign_dn_inv
        } else {
            &ws.sign_dn
        };
        for i3 in (3..L_SUBFR).step_by(STEP) {
            ws.rr_mod[p0] = mult(ctx, Word16(ws.rr_mod[p0]), Word16(psign[i3])).0;
            ws.rr_mod[p1] = mult(ctx, Word16(ws.rr_mod[p1]), Word16(psign[i3 + 1])).0;
            p0 += 1;
            p1 += 1;
        }
    }

    ws.psk = -1;
    ws.alpk = 1;
    ws.ptr_rri0i3_i4 = RRI0I3;
    ws.ptr_rri1i3_i4 = RRI1I3;
    ws.ptr_rri2i3_i4 = RRI2I3;
    ws.ptr_rri3i3_i4 = RRI3I3;
    ws.ip0 = 0;
    ws.ip1 = 1;
    ws.ip2 = 2;
    ws.ip3 = 3;
}
