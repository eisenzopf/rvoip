//! Provenance: Annex B SID previous-frame MA prediction (`LSPGETQ`/`QSIDLSF` lineage).
//! Q-format: LSP/LSF vectors are Q13, MA prediction coefficients remain Q15.

use super::w;
use crate::constants::{M, MA_NP};
use crate::dsp::arith::extract_h;
use crate::dsp::arith32::{l_deposit_l, l_mac, l_msu, l_mult};
use crate::dsp::shift::l_shl;
use crate::dsp::types::DspContext;
use crate::tables::sid::{NOISE_FG_SUM, NOISE_FG_SUM_INV, noise_fg};

pub(super) fn lsp_prev_extract(
    lsp: &[i16; M],
    lsp_ele: &mut [i16; M],
    mode: usize,
    freq_prev: &[[i16; M]; MA_NP],
) {
    let mut ctx = DspContext::default();
    for j in 0..M {
        let mut l_temp = l_shl(&mut ctx, l_deposit_l(w(lsp[j])), 16);
        for (k, freq_row) in freq_prev.iter().enumerate().take(MA_NP) {
            l_temp = l_msu(&mut ctx, l_temp, w(freq_row[j]), w(noise_fg(mode, k, j)));
        }
        let temp = extract_h(l_temp);
        l_temp = l_mult(&mut ctx, temp, w(NOISE_FG_SUM_INV[mode][j]));
        lsp_ele[j] = extract_h(l_shl(&mut ctx, l_temp, 3)).0;
    }
}

pub(super) fn lsp_prev_compose(
    lsp_ele: &[i16; M],
    lsp: &mut [i16; M],
    mode: usize,
    freq_prev: &[[i16; M]; MA_NP],
) {
    let mut ctx = DspContext::default();
    for j in 0..M {
        let mut l_acc = l_mac(
            &mut ctx,
            l_deposit_l(w(0)),
            w(lsp_ele[j]),
            w(NOISE_FG_SUM[mode][j]),
        );
        for (k, freq_row) in freq_prev.iter().enumerate().take(MA_NP) {
            l_acc = l_mac(&mut ctx, l_acc, w(freq_row[j]), w(noise_fg(mode, k, j)));
        }
        lsp[j] = extract_h(l_acc).0;
    }
}

pub(super) fn lsp_prev_update(freq_prev: &mut [[i16; M]; MA_NP], lsp_ele: &[i16; M]) {
    for k in (1..MA_NP).rev() {
        freq_prev[k] = freq_prev[k - 1];
    }
    freq_prev[0] = *lsp_ele;
}
