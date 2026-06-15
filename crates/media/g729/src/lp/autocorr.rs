//! Provenance: LPC/LSP analysis derived from ITU G.729 LPC conversion and recursion routines.
//! Q-format: Correlations and predictor coefficients use Q12/Q13/Q15 fixed-point domains.

#[cfg(feature = "annex_b")]
use crate::constants::NP;
use crate::constants::{L_WINDOW, M};
use crate::dsp::arith::{add, mult_r, sub};
use crate::dsp::arith32::l_mac;
use crate::dsp::oper32::l_extract;
use crate::dsp::shift::{l_shl, norm_l, shr};
use crate::dsp::types::{DspContext, Word16, Word32};
use crate::tables::annexa::HAMWINDOW;

pub(crate) fn autocorr_10(
    x: &[i16; L_WINDOW],
    r_h: &mut [i16; M + 1],
    r_l: &mut [i16; M + 1],
    exp_r0: &mut i16,
) {
    let mut ctx = DspContext::default();
    let mut y = [0i16; L_WINDOW];

    for i in 0..L_WINDOW {
        y[i] = mult_r(&mut ctx, Word16(x[i]), Word16(HAMWINDOW[i])).0;
    }

    *exp_r0 = 1;
    let sum;
    loop {
        ctx.overflow = false;
        let mut s = Word32(1);
        for &yi in &y {
            s = l_mac(&mut ctx, s, Word16(yi), Word16(yi));
        }
        if !ctx.overflow {
            sum = s;
            break;
        }

        for yi in &mut y {
            *yi = shr(&mut ctx, Word16(*yi), 2).0;
        }
        *exp_r0 = add(&mut ctx, Word16(*exp_r0), Word16(4)).0;
    }

    let norm = norm_l(sum);
    let sum_n = l_shl(&mut ctx, sum, norm);
    let (rh0, rl0) = l_extract(sum_n);
    r_h[0] = rh0.0;
    r_l[0] = rl0.0;
    *exp_r0 = sub(&mut ctx, Word16(*exp_r0), Word16(norm)).0;

    for i in 1..=M {
        let mut s = Word32(0);
        for j in 0..(L_WINDOW - i) {
            s = l_mac(&mut ctx, s, Word16(y[j]), Word16(y[j + i]));
        }
        let s = l_shl(&mut ctx, s, norm);
        let (rhi, rli) = l_extract(s);
        r_h[i] = rhi.0;
        r_l[i] = rli.0;
    }
}

#[cfg(feature = "annex_b")]
pub(crate) fn autocorr_np(
    x: &[i16; L_WINDOW],
    r_h: &mut [i16; NP + 1],
    r_l: &mut [i16; NP + 1],
    exp_r0: &mut i16,
) {
    let mut ctx = DspContext::default();
    let mut y = [0i16; L_WINDOW];

    for i in 0..L_WINDOW {
        y[i] = mult_r(&mut ctx, Word16(x[i]), Word16(HAMWINDOW[i])).0;
    }

    *exp_r0 = 1;
    let sum;
    loop {
        ctx.overflow = false;
        let mut s = Word32(1);
        for &yi in &y {
            s = l_mac(&mut ctx, s, Word16(yi), Word16(yi));
        }
        if !ctx.overflow {
            sum = s;
            break;
        }

        for yi in &mut y {
            *yi = shr(&mut ctx, Word16(*yi), 2).0;
        }
        *exp_r0 = add(&mut ctx, Word16(*exp_r0), Word16(4)).0;
    }

    let norm = norm_l(sum);
    let sum_n = l_shl(&mut ctx, sum, norm);
    let (rh0, rl0) = l_extract(sum_n);
    r_h[0] = rh0.0;
    r_l[0] = rl0.0;
    *exp_r0 = sub(&mut ctx, Word16(*exp_r0), Word16(norm)).0;

    for i in 1..=NP {
        let mut s = Word32(0);
        for j in 0..(L_WINDOW - i) {
            s = l_mac(&mut ctx, s, Word16(y[j]), Word16(y[j + i]));
        }
        let s = l_shl(&mut ctx, s, norm);
        let (rhi, rli) = l_extract(s);
        r_h[i] = rhi.0;
        r_l[i] = rli.0;
    }
}
