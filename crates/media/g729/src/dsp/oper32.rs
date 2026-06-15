use crate::dsp::arith::{extract_h, extract_l, mult};
use crate::dsp::arith32::{l_deposit_h, l_mac, l_msu, l_mult, l_sub};
use crate::dsp::div::div_s;
use crate::dsp::shift::{l_shl, l_shr};
use crate::dsp::types::{DspContext, Word16, Word32};

/// Public function `l_extract`.
#[inline(always)]
pub fn l_extract(l_32: Word32) -> (Word16, Word16) {
    let mut ctx = DspContext::default();
    let hi = extract_h(l_32);
    let tmp = l_shr(&mut ctx, l_32, 1);
    let lo = extract_l(l_msu(&mut ctx, tmp, hi, Word16(16384)));
    (hi, lo)
}

/// Public function `l_comp`.
#[inline(always)]
pub fn l_comp(hi: Word16, lo: Word16) -> Word32 {
    let mut ctx = DspContext::default();
    let l = l_deposit_h(hi);
    l_mac(&mut ctx, l, lo, Word16(1))
}

/// Public function `mpy_32`.
#[inline(always)]
pub fn mpy_32(hi1: Word16, lo1: Word16, hi2: Word16, lo2: Word16) -> Word32 {
    let mut ctx = DspContext::default();
    let mut l = l_mult(&mut ctx, hi1, hi2);
    let t1 = mult(&mut ctx, hi1, lo2);
    l = l_mac(&mut ctx, l, t1, Word16(1));
    let t2 = mult(&mut ctx, lo1, hi2);
    l = l_mac(&mut ctx, l, t2, Word16(1));
    l
}

/// Public function `mpy_32_16`.
#[inline(always)]
pub fn mpy_32_16(hi: Word16, lo: Word16, n: Word16) -> Word32 {
    let mut ctx = DspContext::default();
    let mut l = l_mult(&mut ctx, hi, n);
    let t = mult(&mut ctx, lo, n);
    l = l_mac(&mut ctx, l, t, Word16(1));
    l
}

/// Public function `div_32`.
#[inline(always)]
pub fn div_32(l_num: Word32, denom_hi: Word16, denom_lo: Word16) -> Word32 {
    let mut ctx = DspContext::default();

    let approx = div_s(Word16(0x3FFF), denom_hi);

    let mut l_32 = mpy_32_16(denom_hi, denom_lo, approx);
    l_32 = l_sub(&mut ctx, Word32(0x7FFF_FFFF), l_32);

    let (hi, lo) = l_extract(l_32);
    l_32 = mpy_32_16(hi, lo, approx);

    let (hi, lo) = l_extract(l_32);
    let (n_hi, n_lo) = l_extract(l_num);
    l_32 = mpy_32(n_hi, n_lo, hi, lo);

    l_shl(&mut ctx, l_32, 2)
}

/// Public function `L_Extract`.
#[allow(non_snake_case)]
#[inline(always)]
pub fn L_Extract(l_32: Word32) -> (Word16, Word16) {
    l_extract(l_32)
}

/// Public function `L_Comp`.
#[allow(non_snake_case)]
#[inline(always)]
pub fn L_Comp(hi: Word16, lo: Word16) -> Word32 {
    l_comp(hi, lo)
}

/// Public function `Mpy_32`.
#[allow(non_snake_case)]
#[inline(always)]
pub fn Mpy_32(hi1: Word16, lo1: Word16, hi2: Word16, lo2: Word16) -> Word32 {
    mpy_32(hi1, lo1, hi2, lo2)
}

/// Public function `Mpy_32_16`.
#[allow(non_snake_case)]
#[inline(always)]
pub fn Mpy_32_16(hi: Word16, lo: Word16, n: Word16) -> Word32 {
    mpy_32_16(hi, lo, n)
}

/// Public function `Div_32`.
#[allow(non_snake_case)]
#[inline(always)]
pub fn Div_32(l_num: Word32, denom_hi: Word16, denom_lo: Word16) -> Word32 {
    div_32(l_num, denom_hi, denom_lo)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dsp_l_comp_extract_roundtrip_hi() {
        let hi = Word16(10_000);
        let lo = Word16(9_999);
        let x = l_comp(hi, lo);
        let (y_hi, _) = l_extract(x);
        assert_eq!(y_hi, hi);
    }

    #[test]
    fn dsp_div_32_sane_range() {
        let num = Word32(0x1000_0000);
        let den = l_comp(Word16(0x4000), Word16(0));
        let (dhi, dlo) = l_extract(den);
        let q = div_32(num, dhi, dlo);
        assert!(q.0 > 0);
    }
}
