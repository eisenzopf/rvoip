//! Provenance: LPC/LSP analysis derived from ITU G.729 LPC conversion and recursion routines.
//! Q-format: Correlations and predictor coefficients use Q12/Q13/Q15 fixed-point domains.

use crate::constants::{M, MP1};
use crate::dsp::arith::round;
use crate::dsp::arith32::l_mult;
use crate::dsp::types::{DspContext, Word16};

pub(crate) fn weight_az(a: &[i16; MP1], gamma: i16, ap: &mut [i16; MP1]) {
    let mut ctx = DspContext::default();
    ap[0] = a[0];
    let mut fac = gamma;
    for i in 1..M {
        let t = l_mult(&mut ctx, Word16(a[i]), Word16(fac));
        ap[i] = round(&mut ctx, t).0;
        let t2 = l_mult(&mut ctx, Word16(fac), Word16(gamma));
        fac = round(&mut ctx, t2).0;
    }
    let t = l_mult(&mut ctx, Word16(a[M]), Word16(fac));
    ap[M] = round(&mut ctx, t).0;
}

pub(crate) fn weight_az_decode(a: &[i16; MP1], gamma: i16, ap: &mut [i16; MP1]) {
    weight_az(a, gamma, ap);
}
