//! G.729 table-driven transcendentals: `pow2`, `log2` and `inv_sqrt`.
//!
//! These were previously alongside the ETSI basic operators, but they are not
//! basic operators: each reads a G.729 Annex A lookup table, so they belong
//! with G.729 rather than in the shared [`crate::fixed_point`] module.
//!
//! AMR specifies the same three functions (TS 26.073 `oper_32b.c`) over its own
//! tables. Whether those tables are numerically identical to G.729's is an open
//! question, and sharing the code before answering it would be an assumption
//! dressed as reuse — so AMR will bring its own when it needs them.

use crate::codecs::g729::impls::tables::annexa::{TABLOG, TABPOW, TABSQR};
use crate::fixed_point::arith::{extract_h, extract_l};
use crate::fixed_point::arith32::{l_deposit_h, l_msu, l_mult};
use crate::fixed_point::shift::{l_shl, l_shr, l_shr_r, norm_l, shr};
use crate::fixed_point::types::{DspContext, Word16, Word32};

/// Public function `pow2`.
#[inline(always)]
pub fn pow2(exponent: Word16, fraction: Word16) -> Word32 {
    let mut ctx = DspContext::default();
    let mut l_x = l_mult(&mut ctx, fraction, Word16(32));
    let i = extract_h(l_x).0 as usize;
    l_x = l_shr(&mut ctx, l_x, 1);
    let mut a = extract_l(l_x).0;
    a &= 0x7fff;

    let mut out = l_deposit_h(Word16(TABPOW[i]));
    let tmp = Word16(TABPOW[i] - TABPOW[i + 1]);
    out = l_msu(&mut ctx, out, tmp, Word16(a));

    let exp = Word16(30i16.wrapping_sub(exponent.0));
    l_shr_r(&mut ctx, out, exp.0)
}

/// Public function `log2`.
#[inline(always)]
pub fn log2(l_x: Word32) -> (Word16, Word16) {
    if l_x.0 <= 0 {
        return (Word16(0), Word16(0));
    }

    let mut ctx = DspContext::default();
    let exp = norm_l(l_x);
    let mut x = l_shl(&mut ctx, l_x, exp);
    let exponent = Word16(30i16.wrapping_sub(exp));

    x = l_shr(&mut ctx, x, 9);
    let mut i = extract_h(x).0;
    x = l_shr(&mut ctx, x, 1);
    let mut a = extract_l(x).0;
    a &= 0x7fff;
    i = i.wrapping_sub(32);

    let ui = i as usize;
    let mut l_y = l_deposit_h(Word16(TABLOG[ui]));
    let tmp = Word16(TABLOG[ui] - TABLOG[ui + 1]);
    l_y = l_msu(&mut ctx, l_y, tmp, Word16(a));

    (exponent, extract_h(l_y))
}

/// Public function `inv_sqrt`.
#[inline(always)]
pub fn inv_sqrt(l_x: Word32) -> Word32 {
    if l_x.0 <= 0 {
        return Word32(0x3fff_ffff);
    }

    let mut ctx = DspContext::default();
    let mut exp = norm_l(l_x);
    let mut x = l_shl(&mut ctx, l_x, exp);

    exp = 30i16.wrapping_sub(exp);
    if (exp & 1) == 0 {
        x = l_shr(&mut ctx, x, 1);
    }

    exp = shr(&mut ctx, Word16(exp), 1).0;
    exp = exp.wrapping_add(1);

    x = l_shr(&mut ctx, x, 9);
    let mut i = extract_h(x).0;
    x = l_shr(&mut ctx, x, 1);
    let mut a = extract_l(x).0;
    a &= 0x7fff;
    i = i.wrapping_sub(16);

    let ui = i as usize;
    let mut l_y = l_deposit_h(Word16(TABSQR[ui]));
    let tmp = Word16(TABSQR[ui] - TABSQR[ui + 1]);
    l_y = l_msu(&mut ctx, l_y, tmp, Word16(a));
    l_shr(&mut ctx, l_y, exp)
}

/// Public function `Pow2`.
#[allow(non_snake_case)]
#[inline(always)]
pub fn Pow2(exponent: Word16, fraction: Word16) -> Word32 {
    pow2(exponent, fraction)
}

/// Public function `Log2`.
#[allow(non_snake_case)]
#[inline(always)]
pub fn Log2(l_x: Word32, exponent: &mut Word16, fraction: &mut Word16) {
    let (e, f) = log2(l_x);
    *exponent = e;
    *fraction = f;
}

/// Public function `Inv_sqrt`.
#[allow(non_snake_case)]
#[inline(always)]
pub fn Inv_sqrt(l_x: Word32) -> Word32 {
    inv_sqrt(l_x)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dsp_pow2_log2_roundtrip_shape() {
        let x = Word32(0x3000_0000);
        let (e, f) = log2(x);
        let y = pow2(e, f);
        assert!(y.0 > 0);
    }

    #[test]
    fn dsp_inv_sqrt_positive() {
        let y = inv_sqrt(Word32(1 << 20));
        assert!(y.0 > 0);
    }
}
