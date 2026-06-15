//! Provenance: LPC/LSP analysis derived from ITU G.729 LPC conversion and recursion routines.
//! Q-format: Correlations and predictor coefficients use Q12/Q13/Q15 fixed-point domains.

use crate::constants::{M, MP1};
use crate::dsp::arith::add;
use crate::dsp::shift::shr;
use crate::dsp::types::{DspContext, Word16};

pub(crate) fn int_qlpc(lsp_old: &[i16; M], lsp_new: &[i16; M], az: &mut [i16; MP1 * 2]) {
    let mut ctx = DspContext::default();
    let mut lsp = [0i16; M];
    for i in 0..M {
        let a = shr(&mut ctx, Word16(lsp_new[i]), 1);
        let b = shr(&mut ctx, Word16(lsp_old[i]), 1);
        lsp[i] = add(&mut ctx, a, b).0;
    }

    let mut az0 = [0i16; MP1];
    super::lsp_az::lsp_az(&lsp, &mut az0);
    az[..MP1].copy_from_slice(&az0);

    let mut az1 = [0i16; MP1];
    super::lsp_az::lsp_az(lsp_new, &mut az1);
    az[MP1..].copy_from_slice(&az1);
}
