//! Provenance: LPC/LSP analysis derived from ITU G.729 LPC conversion and recursion routines.
//! Q-format: Correlations and predictor coefficients use Q12/Q13/Q15 fixed-point domains.

use crate::constants::{M, MP1};
use crate::dsp::arith::extract_l;
use crate::dsp::arith32::{l_add, l_msu, l_mult, l_sub};
use crate::dsp::oper32::{l_extract, mpy_32_16};
use crate::dsp::shift::l_shr_r;
use crate::dsp::types::{DspContext, Word16, Word32};

#[inline(always)]
fn w(v: i16) -> Word16 {
    Word16(v)
}

#[inline(always)]
fn get_lsp_pol(lsp: &[i16], f: &mut [Word32; 6]) {
    let mut ctx = DspContext::default();
    f[0] = l_mult(&mut ctx, w(4096), w(2048));
    f[1] = l_msu(&mut ctx, Word32(0), w(lsp[0]), w(512));

    let mut f_ptr: isize = 2;
    let mut lsp_ptr = 2usize;

    for i in 2..=5 {
        f[f_ptr as usize] = f[(f_ptr - 2) as usize];
        for _ in 1..i {
            let (hi, lo) = l_extract(f[(f_ptr - 1) as usize]);
            let mut t0 = mpy_32_16(hi, lo, w(lsp[lsp_ptr]));
            t0 = crate::dsp::shift::l_shl(&mut ctx, t0, 1);
            let cur = l_add(&mut ctx, f[f_ptr as usize], f[(f_ptr - 2) as usize]);
            f[f_ptr as usize] = l_sub(&mut ctx, cur, t0);
            f_ptr -= 1;
        }
        f[f_ptr as usize] = l_msu(&mut ctx, f[f_ptr as usize], w(lsp[lsp_ptr]), w(512));
        f_ptr += i as isize;
        lsp_ptr += 2;
    }
}

pub(crate) fn lsp_az(lsp: &[i16; M], a: &mut [i16; MP1]) {
    let mut ctx = DspContext::default();
    let mut f1 = [Word32(0); 6];
    let mut f2 = [Word32(0); 6];

    get_lsp_pol(&lsp[0..], &mut f1);
    get_lsp_pol(&lsp[1..], &mut f2);

    for i in (1..=5).rev() {
        f1[i] = l_add(&mut ctx, f1[i], f1[i - 1]);
        f2[i] = l_sub(&mut ctx, f2[i], f2[i - 1]);
    }

    a[0] = 4096;
    let mut j = 10usize;
    for i in 1..=5 {
        let mut t0 = l_add(&mut ctx, f1[i], f2[i]);
        a[i] = extract_l(l_shr_r(&mut ctx, t0, 13)).0;

        t0 = l_sub(&mut ctx, f1[i], f2[i]);
        a[j] = extract_l(l_shr_r(&mut ctx, t0, 13)).0;
        j -= 1;
    }
}
