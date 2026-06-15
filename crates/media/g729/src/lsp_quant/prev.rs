//! Provenance: LSP quantization/dequantization adapted from ITU G.729 LSP codebook routines.
//! Q-format: LSP/LSF vectors and prediction weights use Q13/Q15 fixed-point representation.

use crate::codec::state::EncoderState;
use crate::constants::{M, MA_NP};
use crate::dsp::arith::extract_h;
use crate::dsp::arith32::{l_mac, l_msu, l_mult};
use crate::dsp::shift::l_shl;
use crate::dsp::types::{DspContext, Word16};
use crate::tables::annexa::{fg, fg_sum, fg_sum_inv};

#[inline(always)]
fn w(v: i16) -> Word16 {
    Word16(v)
}

pub(crate) fn prev_extract_encode(
    state: &EncoderState,
    mode: usize,
    lsp: &[i16; M],
    lsp_ele: &mut [i16; M],
) {
    let mut ctx = DspContext::default();
    for j in 0..M {
        let mut l_temp = crate::dsp::arith32::l_deposit_h(w(lsp[j]));
        for k in 0..MA_NP {
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

pub(crate) fn prev_compose_encode(
    state: &EncoderState,
    mode: usize,
    lsp_ele: &[i16; M],
    lsp: &mut [i16; M],
) {
    let mut ctx = DspContext::default();
    for j in 0..M {
        let mut l_acc = l_mult(&mut ctx, w(lsp_ele[j]), w(fg_sum(mode, j)));
        for k in 0..MA_NP {
            l_acc = l_mac(&mut ctx, l_acc, w(state.freq_prev[k][j]), w(fg(mode, k, j)));
        }
        lsp[j] = extract_h(l_acc).0;
    }
}

pub(crate) fn prev_update_encode(state: &mut EncoderState, lsp_ele: &[i16; M]) {
    for k in (1..MA_NP).rev() {
        state.freq_prev[k] = state.freq_prev[k - 1];
    }
    state.freq_prev[0] = *lsp_ele;
}
