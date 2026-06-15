//! Provenance: LSP quantization/dequantization adapted from ITU G.729 LSP codebook routines.
//! Q-format: LSP/LSF vectors and prediction weights use Q13/Q15 fixed-point representation.

use crate::codec::state::EncoderState;
use crate::constants::{GAP1, GAP2, M, NC, NC0_B, NC1_B};
use crate::dsp::arith::add;
use crate::dsp::shift::shl;
use crate::dsp::types::{DspContext, Word16, Word32};
use crate::tables::annexa::{lspcb1, lspcb2};

#[inline(always)]
fn w(v: i16) -> Word16 {
    Word16(v)
}

pub(crate) fn get_quant(
    state: &mut EncoderState,
    mode: usize,
    code0: usize,
    code1: usize,
    code2: usize,
    lspq: &mut [i16; M],
) {
    let mut ctx = DspContext::default();
    let mut buf = [0i16; M];
    for (j, bj) in buf.iter_mut().enumerate().take(NC) {
        *bj = add(&mut ctx, w(lspcb1(code0, j)), w(lspcb2(code1, j))).0;
    }
    for (j, bj) in buf.iter_mut().enumerate().take(M).skip(NC) {
        *bj = add(&mut ctx, w(lspcb1(code0, j)), w(lspcb2(code2, j))).0;
    }
    super::stability::expand_1_2(&mut buf, GAP1);
    super::stability::expand_1_2(&mut buf, GAP2);
    super::prev::prev_compose_encode(state, mode, &buf, lspq);
    super::prev::prev_update_encode(state, &buf);
    super::stability::stabilize_encode(lspq);
}

pub(crate) fn qua_lsp(
    state: &mut EncoderState,
    lsp: &[i16; M],
    lsp_q: &mut [i16; M],
    ana: &mut [i16; 2],
) {
    let mut ctx = DspContext::default();

    let mut lsf = [0i16; M];
    let mut lsf_q = [0i16; M];
    let mut wegt = [0i16; M];
    crate::lp::lsf::lsp_to_lsf(lsp, &mut lsf);
    super::helpers::get_wegt(&lsf, &mut wegt);

    let mut cand = [0usize; 2];
    let mut tindex1 = [0usize; 2];
    let mut tindex2 = [0usize; 2];
    let mut l_tdist = [Word32(0); 2];

    let mut rbuf = [0i16; M];
    let mut buf = [0i16; M];

    for mode in 0..2 {
        super::prev::prev_extract_encode(state, mode, &lsf, &mut rbuf);
        let mut cand_cur = 0usize;
        super::helpers::pre_select(&rbuf, &mut cand_cur);
        cand[mode] = cand_cur;

        let mut index = 0usize;
        super::helpers::select_1(&rbuf, cand_cur, &wegt, &mut index);
        tindex1[mode] = index;

        for (j, bj) in buf.iter_mut().enumerate().take(NC) {
            *bj = add(&mut ctx, w(lspcb1(cand_cur, j)), w(lspcb2(index, j))).0;
        }
        super::stability::expand_1(&mut buf, GAP1);

        super::helpers::select_2(&rbuf, cand_cur, &wegt, &mut index);
        tindex2[mode] = index;

        for (j, bj) in buf.iter_mut().enumerate().take(M).skip(NC) {
            *bj = add(&mut ctx, w(lspcb1(cand_cur, j)), w(lspcb2(index, j))).0;
        }
        super::stability::expand_2(&mut buf, GAP1);
        super::stability::expand_1_2(&mut buf, GAP2);

        super::helpers::get_tdist(&wegt, &buf, &rbuf, mode, &mut l_tdist[mode]);
    }

    let mut mode_index = 0usize;
    super::helpers::last_select(&l_tdist, &mut mode_index);

    ana[0] = shl(&mut ctx, w(mode_index as i16), NC0_B as i16).0 | cand[mode_index] as i16;
    ana[1] =
        shl(&mut ctx, w(tindex1[mode_index] as i16), NC1_B as i16).0 | tindex2[mode_index] as i16;

    get_quant(
        state,
        mode_index,
        cand[mode_index],
        tindex1[mode_index],
        tindex2[mode_index],
        &mut lsf_q,
    );

    crate::lp::lsf::lsf_to_lsp(&lsf_q, lsp_q);
}
