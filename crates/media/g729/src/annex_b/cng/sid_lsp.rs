//! Annex B SID LSP-domain helpers.
//! Provenance: Annex B VAD/DTX/CNG behavior adapted from ITU G.729 Annex B reference routines.
//! Q-format: VAD features, SID parameters, and CNG gains use Q0/Q13/Q15 fixed-point domains.

pub(super) use super::sid_lsp_prev::{lsp_prev_compose, lsp_prev_extract, lsp_prev_update};
use super::w;
use crate::constants::{CONST10, CONST12, GAP3, L_LIMIT, M, M_LIMIT, MA_NP, PI04, PI92};
use crate::dsp::arith::{add, extract_h, extract_l, mult, sub};
use crate::dsp::arith32::{l_deposit_l, l_mac, l_mult, l_sub};
use crate::dsp::shift::{l_shl, l_shr, norm_s, shl, shr};
use crate::dsp::types::DspContext;
use crate::tables::annexa::{SLOPE_ACOS, SLOPE_COS, TABLE2, lspcb1, lspcb2};
use crate::tables::sid::{PTR_TAB_1, PTR_TAB_2};

pub(super) fn lsp_lsf2(lsp: &[i16; M], lsf: &mut [i16; M]) {
    let mut ctx = DspContext::default();
    let mut ind = 63i16;
    for i in (0..M).rev() {
        while sub(&mut ctx, w(TABLE2[ind as usize]), w(lsp[i])).0 < 0 {
            ind = sub(&mut ctx, w(ind), w(1)).0;
            if ind <= 0 {
                break;
            }
        }
        let offset = sub(&mut ctx, w(lsp[i]), w(TABLE2[ind as usize]));
        let mut l_tmp = l_mult(&mut ctx, w(SLOPE_ACOS[ind as usize]), offset);
        l_tmp = l_shr(&mut ctx, l_tmp, 12);
        let ind_q = shl(&mut ctx, w(ind), 9);
        let freq = add(&mut ctx, ind_q, extract_l(l_tmp));
        lsf[i] = mult(&mut ctx, freq, w(25736)).0;
    }
}

pub(super) fn lsf_lsp2(lsf: &[i16; M], lsp: &mut [i16; M]) {
    let mut ctx = DspContext::default();
    for i in 0..M {
        let freq = mult(&mut ctx, w(lsf[i]), w(20861)).0;
        let mut ind = shr(&mut ctx, w(freq), 8).0;
        let offset = freq & 0x00ff;
        if sub(&mut ctx, w(ind), w(63)).0 > 0 {
            ind = 63;
        }
        let li = ind as usize;
        let mut l_tmp = l_mult(&mut ctx, w(SLOPE_COS[li]), w(offset));
        l_tmp = l_shr(&mut ctx, l_tmp, 13);
        lsp[i] = add(&mut ctx, w(TABLE2[li]), extract_l(l_tmp)).0;
    }
}

pub(super) fn lsp_expand_1_2(buf: &mut [i16; M], gap: i16) {
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

pub(super) fn lsp_stability(buf: &mut [i16; M]) {
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

pub(super) fn get_wegt(flsp: &[i16; M], wegt: &mut [i16; M]) {
    let mut ctx = DspContext::default();
    let mut buf = [0i16; M];

    buf[0] = sub(&mut ctx, w(flsp[1]), w(PI04 + 8192)).0;
    for i in 1..(M - 1) {
        let tmp = sub(&mut ctx, w(flsp[i + 1]), w(flsp[i - 1])).0;
        buf[i] = sub(&mut ctx, w(tmp), w(8192)).0;
    }
    buf[M - 1] = sub(&mut ctx, w(PI92 - 8192), w(flsp[M - 2])).0;

    for i in 0..M {
        if buf[i] > 0 {
            wegt[i] = 2048;
        } else {
            let mut l_acc = l_mult(&mut ctx, w(buf[i]), w(buf[i]));
            let mut tmp = extract_h(l_shl(&mut ctx, l_acc, 2)).0;
            l_acc = l_mult(&mut ctx, w(tmp), w(CONST10));
            tmp = extract_h(l_shl(&mut ctx, l_acc, 2)).0;
            wegt[i] = add(&mut ctx, w(tmp), w(2048)).0;
        }
    }

    let mut l_acc = l_mult(&mut ctx, w(wegt[4]), w(CONST12));
    wegt[4] = extract_h(l_shl(&mut ctx, l_acc, 1)).0;
    l_acc = l_mult(&mut ctx, w(wegt[5]), w(CONST12));
    wegt[5] = extract_h(l_shl(&mut ctx, l_acc, 1)).0;

    let mut tmp = 0i16;
    for &v in wegt.iter() {
        if sub(&mut ctx, w(v), w(tmp)).0 > 0 {
            tmp = v;
        }
    }
    let sft = norm_s(w(tmp));
    for wv in wegt.iter_mut() {
        *wv = shl(&mut ctx, w(*wv), sft).0;
    }
}
/// Public function `sid_lsfq_decode`.
pub fn sid_lsfq_decode(index: &[i16], lspq: &mut [i16; M], freq_prev: &mut [[i16; M]; MA_NP]) {
    let mut ctx = DspContext::default();
    let mut lsfq = [0i16; M];
    let mut tmpbuf = [0i16; M];

    for (i, t) in tmpbuf.iter_mut().enumerate().take(M) {
        *t = lspcb1(PTR_TAB_1[index[1] as usize] as usize, i);
    }
    for i in 0..(M / 2) {
        tmpbuf[i] = add(
            &mut ctx,
            w(tmpbuf[i]),
            w(lspcb2(PTR_TAB_2[0][index[2] as usize] as usize, i)),
        )
        .0;
    }
    for i in (M / 2)..M {
        tmpbuf[i] = add(
            &mut ctx,
            w(tmpbuf[i]),
            w(lspcb2(PTR_TAB_2[1][index[2] as usize] as usize, i)),
        )
        .0;
    }

    for j in 1..M {
        let mut acc0 = l_mult(&mut ctx, w(tmpbuf[j - 1]), w(16384));
        acc0 = l_mac(&mut ctx, acc0, w(tmpbuf[j]), w(-16384));
        acc0 = l_mac(&mut ctx, acc0, w(10), w(16384));
        let k = extract_h(acc0).0;
        if k > 0 {
            tmpbuf[j - 1] = sub(&mut ctx, w(tmpbuf[j - 1]), w(k)).0;
            tmpbuf[j] = add(&mut ctx, w(tmpbuf[j]), w(k)).0;
        }
    }

    lsp_prev_compose(&tmpbuf, &mut lsfq, index[0] as usize, freq_prev);
    lsp_prev_update(freq_prev, &tmpbuf);
    lsp_stability(&mut lsfq);
    lsf_lsp2(&lsfq, lspq);
}
