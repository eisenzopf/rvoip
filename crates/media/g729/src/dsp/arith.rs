use crate::dsp::arith32::l_add;
use crate::dsp::types::{DspContext, MAX_16, MIN_16, Word16, Word32};

/// Public function `sature`.
#[inline(always)]
pub fn sature(ctx: &mut DspContext, l_var1: Word32) -> Word16 {
    if l_var1.0 > i32::from(MAX_16) {
        ctx.overflow = true;
        Word16(MAX_16)
    } else if l_var1.0 < i32::from(MIN_16) {
        ctx.overflow = true;
        Word16(MIN_16)
    } else {
        ctx.overflow = false;
        Word16(l_var1.0 as i16)
    }
}

/// Public function `add`.
#[inline(always)]
pub fn add(ctx: &mut DspContext, var1: Word16, var2: Word16) -> Word16 {
    let l_sum = i32::from(var1.0) + i32::from(var2.0);
    sature(ctx, Word32(l_sum))
}

/// Public function `sub`.
#[inline(always)]
pub fn sub(ctx: &mut DspContext, var1: Word16, var2: Word16) -> Word16 {
    let l_diff = i32::from(var1.0) - i32::from(var2.0);
    sature(ctx, Word32(l_diff))
}

/// Public function `abs_s`.
#[inline(always)]
pub fn abs_s(_ctx: &mut DspContext, var1: Word16) -> Word16 {
    if var1.0 == MIN_16 {
        Word16(MAX_16)
    } else if var1.0 < 0 {
        Word16(-var1.0)
    } else {
        var1
    }
}

/// Public function `negate`.
#[inline(always)]
pub fn negate(_ctx: &mut DspContext, var1: Word16) -> Word16 {
    if var1.0 == MIN_16 {
        Word16(MAX_16)
    } else {
        Word16(-var1.0)
    }
}

/// Public constant `fn`.
#[inline(always)]
pub const fn extract_h(l_var1: Word32) -> Word16 {
    Word16((l_var1.0 >> 16) as i16)
}

/// Public constant `fn`.
#[inline(always)]
pub const fn extract_l(l_var1: Word32) -> Word16 {
    Word16(l_var1.0 as i16)
}

/// Public function `round`.
#[inline(always)]
pub fn round(ctx: &mut DspContext, l_var1: Word32) -> Word16 {
    let l_arrondi = l_add(ctx, l_var1, Word32(0x0000_8000));
    extract_h(l_arrondi)
}

/// Public function `mult`.
#[inline(always)]
pub fn mult(ctx: &mut DspContext, var1: Word16, var2: Word16) -> Word16 {
    let p = i32::from(var1.0).wrapping_mul(i32::from(var2.0)) as u32;
    let mut l_prod = ((p & 0xFFFF_8000) >> 15) as i32;
    if (l_prod & 0x0001_0000) != 0 {
        l_prod |= !0x0000_FFFF;
    }
    sature(ctx, Word32(l_prod))
}

/// Public function `mult_r`.
#[inline(always)]
pub fn mult_r(ctx: &mut DspContext, var1: Word16, var2: Word16) -> Word16 {
    let p = i32::from(var1.0).wrapping_mul(i32::from(var2.0));
    let mut l_prod = (p.wrapping_add(0x0000_4000) as u32) & 0xFFFF_8000;
    l_prod >>= 15;

    let mut signed = l_prod as i32;
    if (signed & 0x0001_0000) != 0 {
        signed |= !0x0000_FFFF;
    }
    sature(ctx, Word32(signed))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dsp_add_saturates() {
        let mut ctx = DspContext::default();
        let r = add(&mut ctx, Word16(MAX_16), Word16(1));
        assert_eq!(r.0, MAX_16);
        assert!(ctx.overflow);
    }

    #[test]
    fn dsp_mult_corner_min_min() {
        let mut ctx = DspContext::default();
        let r = mult(&mut ctx, Word16(MIN_16), Word16(MIN_16));
        assert_eq!(r.0, MAX_16);
        assert!(ctx.overflow);
    }

    #[test]
    fn dsp_round_matches_reference_shape() {
        let mut ctx = DspContext::default();
        let r = round(&mut ctx, Word32(0x0001_7FFF));
        assert_eq!(r.0, 1);
        let r2 = round(&mut ctx, Word32(0x0001_8000));
        assert_eq!(r2.0, 2);
    }
}
