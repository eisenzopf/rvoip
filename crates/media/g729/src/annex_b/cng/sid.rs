//! Annex B SID quantization/dequantization integration.
//! Provenance: Annex B VAD/DTX/CNG behavior adapted from ITU G.729 Annex B reference routines.
//! Q-format: VAD features, SID parameters, and CNG gains use Q0/Q13/Q15 fixed-point domains.

use super::{
    sid_lsp::{
        get_wegt, lsf_lsp2, lsp_expand_1_2, lsp_lsf2, lsp_prev_compose, lsp_prev_extract,
        lsp_prev_update, lsp_stability,
    },
    sid_quant::qnt_e,
    w,
};
use crate::constants::{GAP3, L_LIMIT, M, M_LIMIT, MA_NP};
use crate::dsp::arith::{add, sub};
use crate::dsp::types::DspContext;

/// Public re-export.
pub use super::sid_lsp::sid_lsfq_decode;
/// Public re-export.
pub use super::sid_quant::qua_sidgain;

/// Public function `lsfq_noise`.
pub fn lsfq_noise(
    lsp: &[i16; M],
    lspq: &mut [i16; M],
    freq_prev: &mut [[i16; M]; MA_NP],
    ana: &mut [i16; 3],
) {
    let mut ctx = DspContext::default();

    let mut lsf = [0i16; M];
    let mut lsfq = [0i16; M];
    let mut weight = [0i16; M];
    let mut tmpbuf = [0i16; M];
    let mut errlsf = [0i16; 2 * M];

    lsp_lsf2(lsp, &mut lsf);

    if lsf[0] < L_LIMIT {
        lsf[0] = L_LIMIT;
    }
    for i in 0..(M - 1) {
        if sub(&mut ctx, w(lsf[i + 1]), w(lsf[i])).0 < 2 * GAP3 {
            lsf[i + 1] = add(&mut ctx, w(lsf[i]), w(2 * GAP3)).0;
        }
    }
    if lsf[M - 1] > M_LIMIT {
        lsf[M - 1] = M_LIMIT;
    }
    if lsf[M - 1] < lsf[M - 2] {
        lsf[M - 2] = sub(&mut ctx, w(lsf[M - 1]), w(GAP3)).0;
    }

    get_wegt(&lsf, &mut weight);

    for mode in 0..2 {
        let mut one = [0i16; M];
        lsp_prev_extract(&lsf, &mut one, mode, freq_prev);
        errlsf[mode * M..(mode + 1) * M].copy_from_slice(&one);
    }

    let mut mode = 0i16;
    let mut clust = [0i16; 2];
    qnt_e(
        &errlsf,
        &weight,
        2,
        &mut tmpbuf,
        &mut mode,
        1,
        &mut clust,
        &[32, 16],
    );
    ana[0] = mode;
    ana[1] = clust[0];
    ana[2] = clust[1];

    lsp_expand_1_2(&mut tmpbuf, 10);

    lsp_prev_compose(&tmpbuf, &mut lsfq, mode as usize, freq_prev);
    lsp_prev_update(freq_prev, &tmpbuf);
    lsp_stability(&mut lsfq);
    lsf_lsp2(&lsfq, lspq);
}
