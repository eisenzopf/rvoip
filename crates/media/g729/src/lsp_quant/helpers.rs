//! Provenance: LSP quantization/dequantization adapted from ITU G.729 LSP codebook routines.
//! Q-format: LSP/LSF vectors and prediction weights use Q13/Q15 fixed-point representation.

use crate::constants::{CONST10, CONST12, M, NC, NC0, NC1, PI04, PI92};
use crate::dsp::arith::{add, extract_h, mult, sub};
use crate::dsp::arith32::{l_mac, l_mult, l_sub};
use crate::dsp::shift::{l_shl, norm_s, shl};
use crate::dsp::types::{DspContext, MAX_32, Word16, Word32};
use crate::tables::annexa::{fg_sum, lspcb1, lspcb2};

#[inline(always)]
fn w(v: i16) -> Word16 {
    Word16(v)
}

pub(crate) fn get_wegt(flsp: &[i16; M], wegt: &mut [i16; M]) {
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

pub(crate) fn pre_select(rbuf: &[i16; M], cand: &mut usize) {
    let mut ctx = DspContext::default();
    *cand = 0;
    let mut l_dmin = Word32(MAX_32);
    for i in 0..NC0 {
        let mut l_tmp = Word32(0);
        for (j, &rv) in rbuf.iter().enumerate().take(M) {
            let tmp = sub(&mut ctx, w(rv), w(lspcb1(i, j))).0;
            l_tmp = l_mac(&mut ctx, l_tmp, w(tmp), w(tmp));
        }
        if l_sub(&mut ctx, l_tmp, l_dmin).0 < 0 {
            l_dmin = l_tmp;
            *cand = i;
        }
    }
}

pub(crate) fn select_1(rbuf: &[i16; M], c0: usize, wegt: &[i16; M], index: &mut usize) {
    let mut ctx = DspContext::default();
    let mut buf = [0i16; M];
    for j in 0..NC {
        buf[j] = sub(&mut ctx, w(rbuf[j]), w(lspcb1(c0, j))).0;
    }
    *index = 0;
    let mut l_dmin = Word32(MAX_32);
    for k1 in 0..NC1 {
        let mut l_dist = Word32(0);
        for j in 0..NC {
            let tmp = sub(&mut ctx, w(buf[j]), w(lspcb2(k1, j))).0;
            let tmp2 = mult(&mut ctx, w(wegt[j]), w(tmp)).0;
            l_dist = l_mac(&mut ctx, l_dist, w(tmp2), w(tmp));
        }
        if l_sub(&mut ctx, l_dist, l_dmin).0 < 0 {
            l_dmin = l_dist;
            *index = k1;
        }
    }
}

pub(crate) fn select_2(rbuf: &[i16; M], c0: usize, wegt: &[i16; M], index: &mut usize) {
    let mut ctx = DspContext::default();
    let mut buf = [0i16; M];
    for j in NC..M {
        buf[j] = sub(&mut ctx, w(rbuf[j]), w(lspcb1(c0, j))).0;
    }
    *index = 0;
    let mut l_dmin = Word32(MAX_32);
    for k1 in 0..NC1 {
        let mut l_dist = Word32(0);
        for j in NC..M {
            let tmp = sub(&mut ctx, w(buf[j]), w(lspcb2(k1, j))).0;
            let tmp2 = mult(&mut ctx, w(wegt[j]), w(tmp)).0;
            l_dist = l_mac(&mut ctx, l_dist, w(tmp2), w(tmp));
        }
        if l_sub(&mut ctx, l_dist, l_dmin).0 < 0 {
            l_dmin = l_dist;
            *index = k1;
        }
    }
}

pub(crate) fn get_tdist(
    wegt: &[i16; M],
    buf: &[i16; M],
    rbuf: &[i16; M],
    mode: usize,
    l_tdist: &mut Word32,
) {
    let mut ctx = DspContext::default();
    *l_tdist = Word32(0);
    for j in 0..M {
        let mut tmp = sub(&mut ctx, w(buf[j]), w(rbuf[j])).0;
        tmp = mult(&mut ctx, w(tmp), w(fg_sum(mode, j))).0;
        let l_acc = l_mult(&mut ctx, w(wegt[j]), w(tmp));
        let tmp2 = extract_h(l_shl(&mut ctx, l_acc, 4)).0;
        *l_tdist = l_mac(&mut ctx, *l_tdist, w(tmp2), w(tmp));
    }
}

pub(crate) fn last_select(l_tdist: &[Word32; 2], mode_index: &mut usize) {
    let mut ctx = DspContext::default();
    *mode_index = 0;
    if l_sub(&mut ctx, l_tdist[1], l_tdist[0]).0 < 0 {
        *mode_index = 1;
    }
}
