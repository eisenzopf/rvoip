//! Provenance: LPC/LSP analysis derived from ITU G.729 LPC conversion and recursion routines.
//! Q-format: Correlations and predictor coefficients use Q12/Q13/Q15 fixed-point domains.

use crate::constants::M;
#[cfg(feature = "annex_b")]
use crate::dsp::arith::round;
use crate::dsp::arith::{add, mult, sub};
use crate::dsp::arith32::l_mult;
#[cfg(feature = "annex_b")]
use crate::dsp::shift::l_shl;
use crate::dsp::shift::{l_shr, shl, shr};
use crate::dsp::types::{DspContext, Word16};
use crate::tables::annexa::{SLOPE_ACOS, SLOPE_COS, TABLE2};

#[inline(always)]
fn w(v: i16) -> Word16 {
    Word16(v)
}

pub(crate) fn lsp_to_lsf(lsp: &[i16; M], lsf: &mut [i16; M]) {
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
        let freq = add(&mut ctx, ind_q, crate::dsp::arith::extract_l(l_tmp));
        lsf[i] = mult(&mut ctx, freq, w(25736)).0;
    }
}

pub(crate) fn lsf_to_lsp(lsf: &[i16; M], lsp: &mut [i16; M]) {
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
        lsp[i] = add(&mut ctx, w(TABLE2[li]), crate::dsp::arith::extract_l(l_tmp)).0;
    }
}

#[cfg(feature = "annex_b")]
pub(crate) fn lsp_to_lsf_annex_b(lsp: &[i16; M], lsf: &mut [i16; M]) {
    let mut ctx = DspContext::default();
    let mut ind = 63i16;

    for i in (0..M).rev() {
        while sub(&mut ctx, w(TABLE2[ind as usize]), w(lsp[i])).0 < 0 {
            ind = sub(&mut ctx, w(ind), w(1)).0;
        }

        let offset = sub(&mut ctx, w(lsp[i]), w(TABLE2[ind as usize]));
        let mut l_tmp = l_mult(&mut ctx, offset, w(SLOPE_ACOS[ind as usize]));
        l_tmp = l_shl(&mut ctx, l_tmp, 3);
        let tmp = round(&mut ctx, l_tmp);
        let ind_q = shl(&mut ctx, w(ind), 8);
        lsf[i] = add(&mut ctx, tmp, ind_q).0;
    }
}
