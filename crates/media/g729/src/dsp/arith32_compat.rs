use crate::dsp::arith::extract_h;
use crate::dsp::types::{DspContext, MAX_32, MIN_32, Word16, Word32};

use super::{l_add, l_mac, l_msu, l_mult, l_sub};

/// Public function `mac_r`.
#[inline(always)]
pub fn mac_r(ctx: &mut DspContext, l_var3: Word32, var1: Word16, var2: Word16) -> Word16 {
    let l = l_mac(ctx, l_var3, var1, var2);
    let l = l_add(ctx, l, Word32(0x0000_8000));
    extract_h(l)
}

/// Public function `msu_r`.
#[inline(always)]
pub fn msu_r(ctx: &mut DspContext, l_var3: Word32, var1: Word16, var2: Word16) -> Word16 {
    let l = l_msu(ctx, l_var3, var1, var2);
    let l = l_add(ctx, l, Word32(0x0000_8000));
    extract_h(l)
}

/// Public function `l_abs`.
#[inline(always)]
pub fn l_abs(_ctx: &mut DspContext, l_var1: Word32) -> Word32 {
    if l_var1.0 == MIN_32 {
        Word32(MAX_32)
    } else if l_var1.0 < 0 {
        Word32(-l_var1.0)
    } else {
        l_var1
    }
}

/// Public function `l_sat`.
#[inline(always)]
pub fn l_sat(ctx: &mut DspContext, l_var1: Word32) -> Word32 {
    let mut out = l_var1;
    if ctx.overflow {
        if ctx.carry {
            out = Word32(MIN_32);
        } else {
            out = Word32(MAX_32);
        }
        ctx.carry = false;
        ctx.overflow = false;
    }
    out
}

/// Public function `L_mult`.
#[allow(non_snake_case)]
#[inline(always)]
pub fn L_mult(ctx: &mut DspContext, var1: Word16, var2: Word16) -> Word32 {
    l_mult(ctx, var1, var2)
}

/// Public function `L_add`.
#[allow(non_snake_case)]
#[inline(always)]
pub fn L_add(ctx: &mut DspContext, l_var1: Word32, l_var2: Word32) -> Word32 {
    l_add(ctx, l_var1, l_var2)
}

/// Public function `L_sub`.
#[allow(non_snake_case)]
#[inline(always)]
pub fn L_sub(ctx: &mut DspContext, l_var1: Word32, l_var2: Word32) -> Word32 {
    l_sub(ctx, l_var1, l_var2)
}

/// Public function `L_mac`.
#[allow(non_snake_case)]
#[inline(always)]
pub fn L_mac(ctx: &mut DspContext, l_var3: Word32, var1: Word16, var2: Word16) -> Word32 {
    l_mac(ctx, l_var3, var1, var2)
}

/// Public function `L_msu`.
#[allow(non_snake_case)]
#[inline(always)]
pub fn L_msu(ctx: &mut DspContext, l_var3: Word32, var1: Word16, var2: Word16) -> Word32 {
    l_msu(ctx, l_var3, var1, var2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dsp_l_add_overflow_sets_flag() {
        let mut ctx = DspContext::default();
        let _ = l_add(&mut ctx, Word32(MAX_32), Word32(1));
        assert!(ctx.overflow);
    }

    #[test]
    fn dsp_l_mult_min_min_saturates() {
        let mut ctx = DspContext::default();
        let r = l_mult(&mut ctx, Word16(i16::MIN), Word16(i16::MIN));
        assert_eq!(r.0, MAX_32);
        assert!(ctx.overflow);
    }

    #[test]
    fn dsp_l_add_c_carry_paths() {
        let mut ctx = DspContext {
            overflow: false,
            carry: true,
        };
        let _ = super::super::l_add_c(&mut ctx, Word32(MAX_32), Word32(0));
        assert!(ctx.overflow);
    }
}
