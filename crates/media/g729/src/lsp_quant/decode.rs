//! Provenance: LSP quantization/dequantization adapted from ITU G.729 LSP codebook routines.
//! Q-format: LSP/LSF vectors and prediction weights use Q13/Q15 fixed-point representation.

use crate::codec::state::DecoderState;
use crate::constants::{GAP1, GAP2, GAP3, L_LIMIT, M, M_LIMIT, NC, NC0, NC0_B, NC1, NC1_B};
use crate::dsp::arith::{add, extract_h, sub};
use crate::dsp::arith32::{l_deposit_h, l_deposit_l, l_mac, l_msu, l_mult, l_sub};
use crate::dsp::shift::{l_shl, shr};
use crate::dsp::types::{DspContext, Word16};
use crate::tables::annexa::{fg, fg_sum, fg_sum_inv, lspcb1, lspcb2};

#[inline(always)]
fn w(v: i16) -> Word16 {
    Word16(v)
}

pub(crate) fn lsp_expand_1_2(buf: &mut [i16; M], gap: i16) {
    let mut ctx = DspContext::default();
    for j in 1..M {
        let diff = sub(&mut ctx, w(buf[j - 1]), w(buf[j]));
        let sum = add(&mut ctx, diff, w(gap));
        let tmp = shr(&mut ctx, sum, 1);
        if tmp.0 > 0 {
            buf[j - 1] = sub(&mut ctx, w(buf[j - 1]), tmp).0;
            buf[j] = add(&mut ctx, w(buf[j]), tmp).0;
        }
    }
}

pub(crate) fn lsp_prev_compose(
    state: &DecoderState,
    mode: usize,
    lsp_ele: &[i16; M],
    lsp: &mut [i16; M],
) {
    let mut ctx = DspContext::default();
    for j in 0..M {
        let mut l_acc = l_mult(&mut ctx, w(lsp_ele[j]), w(fg_sum(mode, j)));
        for k in 0..4 {
            l_acc = l_mac(&mut ctx, l_acc, w(state.freq_prev[k][j]), w(fg(mode, k, j)));
        }
        lsp[j] = extract_h(l_acc).0;
    }
}

pub(crate) fn lsp_prev_extract(
    state: &DecoderState,
    mode: usize,
    lsp: &[i16; M],
    lsp_ele: &mut [i16; M],
) {
    let mut ctx = DspContext::default();
    for j in 0..M {
        let mut l_temp = l_deposit_h(w(lsp[j]));
        for k in 0..4 {
            l_temp = l_msu(
                &mut ctx,
                l_temp,
                w(state.freq_prev[k][j]),
                w(fg(mode, k, j)),
            );
        }
        let temp = extract_h(l_temp);
        l_temp = l_mult(&mut ctx, temp, w(fg_sum_inv(mode, j)));
        lsp_ele[j] = extract_h(l_shl(&mut ctx, l_temp, 3)).0;
    }
}

pub(crate) fn lsp_prev_update(state: &mut DecoderState, lsp_ele: &[i16; M]) {
    for k in (1..4).rev() {
        state.freq_prev[k] = state.freq_prev[k - 1];
    }
    state.freq_prev[0] = *lsp_ele;
}

pub(crate) fn lsp_stability(buf: &mut [i16; M]) {
    let mut ctx = DspContext::default();

    for j in 0..(M - 1) {
        let l_diff = l_sub(&mut ctx, l_deposit_l(w(buf[j + 1])), l_deposit_l(w(buf[j])));
        if l_diff.0 < 0 {
            buf.swap(j, j + 1);
        }
    }

    if sub(&mut ctx, w(buf[0]), w(L_LIMIT)).0 < 0 {
        buf[0] = L_LIMIT;
    }
    for j in 0..(M - 1) {
        let l_diff = l_sub(&mut ctx, l_deposit_l(w(buf[j + 1])), l_deposit_l(w(buf[j])));
        if l_sub(&mut ctx, l_diff, l_deposit_l(w(GAP3))).0 < 0 {
            buf[j + 1] = add(&mut ctx, w(buf[j]), w(GAP3)).0;
        }
    }
    if sub(&mut ctx, w(buf[M - 1]), w(M_LIMIT)).0 > 0 {
        buf[M - 1] = M_LIMIT;
    }
}

fn lsp_get_quant(
    state: &mut DecoderState,
    mode: usize,
    code0: usize,
    code1: usize,
    code2: usize,
    lspq: &mut [i16; M],
) {
    let mut buf = [0i16; M];
    let mut ctx = DspContext::default();
    for (j, bj) in buf.iter_mut().enumerate().take(NC) {
        *bj = add(&mut ctx, w(lspcb1(code0, j)), w(lspcb2(code1, j))).0;
    }
    for (j, bj) in buf.iter_mut().enumerate().take(M).skip(NC) {
        *bj = add(&mut ctx, w(lspcb1(code0, j)), w(lspcb2(code2, j))).0;
    }

    lsp_expand_1_2(&mut buf, GAP1);
    lsp_expand_1_2(&mut buf, GAP2);
    lsp_prev_compose(state, mode, &buf, lspq);
    lsp_prev_update(state, &buf);
    lsp_stability(lspq);
}

fn lsp_iqua_cs(state: &mut DecoderState, prm: &[i16], lsp_q: &mut [i16; M], erase: i16) {
    let mut ctx = DspContext::default();
    if erase == 0 {
        let mode_index = (shr(&mut ctx, w(prm[0]), NC0_B as i16).0 & 1) as usize;
        let code0 = (prm[0] & (NC0 as i16 - 1)) as usize;
        let code1 = (shr(&mut ctx, w(prm[1]), NC1_B as i16).0 & (NC1 as i16 - 1)) as usize;
        let code2 = (prm[1] & (NC1 as i16 - 1)) as usize;

        lsp_get_quant(state, mode_index, code0, code1, code2, lsp_q);
        state.prev_lsp = *lsp_q;
        state.prev_ma = mode_index as i16;
    } else {
        *lsp_q = state.prev_lsp;
        let mut buf = [0i16; M];
        lsp_prev_extract(state, state.prev_ma as usize, &state.prev_lsp, &mut buf);
        lsp_prev_update(state, &buf);
    }
}

pub(crate) fn d_lsp(state: &mut DecoderState, prm: &[i16], lsp_q: &mut [i16; M], erase: i16) {
    let mut lsf_q = [0i16; M];
    lsp_iqua_cs(state, prm, &mut lsf_q, erase);
    crate::lp::lsf::lsf_to_lsp(&lsf_q, lsp_q);
}
