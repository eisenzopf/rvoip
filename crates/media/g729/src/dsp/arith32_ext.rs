//! Provenance: ITU STL extended carry/borrow arithmetic (`L_add_c`/`L_sub_c`).
//! Q-format: operations preserve 32-bit fixed-point accumulators with carry state.

use crate::dsp::types::{DspContext, MAX_32, MIN_32, Word16, Word32};

use super::l_mult;

/// Public function `l_add_c`.
#[inline(always)]
pub fn l_add_c(ctx: &mut DspContext, l_var1: Word32, l_var2: Word32) -> Word32 {
    let x = l_var1.0;
    let y = l_var2.0;
    let carry_in = if ctx.carry { 1 } else { 0 };

    let out = x.wrapping_add(y).wrapping_add(carry_in);
    let l_test = x.wrapping_add(y);
    let carry_int: bool;

    if (x > 0) && (y > 0) && (l_test < 0) {
        ctx.overflow = true;
        carry_int = false;
    } else if (x < 0) && (y < 0) && (l_test > 0) {
        ctx.overflow = true;
        carry_int = true;
    } else if ((x ^ y) < 0) && (l_test > 0) {
        ctx.overflow = false;
        carry_int = true;
    } else {
        ctx.overflow = false;
        carry_int = false;
    }

    if ctx.carry {
        if l_test == MAX_32 {
            ctx.overflow = true;
            ctx.carry = carry_int;
        } else if l_test == -1 {
            ctx.carry = true;
        } else {
            ctx.carry = carry_int;
        }
    } else {
        ctx.carry = carry_int;
    }

    Word32(out)
}

/// Public function `l_sub_c`.
#[inline(always)]
pub fn l_sub_c(ctx: &mut DspContext, l_var1: Word32, l_var2: Word32) -> Word32 {
    let x = l_var1.0;
    let y = l_var2.0;
    let out: i32;

    if ctx.carry {
        ctx.carry = false;
        if y != MIN_32 {
            out = l_add_c(ctx, Word32(x), Word32(-y)).0;
        } else {
            out = x.wrapping_sub(y);
            if x > 0 {
                ctx.overflow = true;
                ctx.carry = false;
            }
        }
    } else {
        out = x.wrapping_sub(y).wrapping_sub(1);
        let l_test = x.wrapping_sub(y);
        let mut carry_int = false;

        if (l_test < 0) && (x > 0) && (y < 0) {
            ctx.overflow = true;
            carry_int = false;
        } else if (l_test > 0) && (x < 0) && (y > 0) {
            ctx.overflow = true;
            carry_int = true;
        } else if (l_test > 0) && ((x ^ y) > 0) {
            ctx.overflow = false;
            carry_int = true;
        }

        if l_test == MIN_32 {
            ctx.overflow = true;
            ctx.carry = carry_int;
        } else {
            ctx.carry = carry_int;
        }
    }

    Word32(out)
}

/// Public function `l_mac_ns`.
#[inline(always)]
pub fn l_mac_ns(ctx: &mut DspContext, l_var3: Word32, var1: Word16, var2: Word16) -> Word32 {
    let p = l_mult(ctx, var1, var2);
    l_add_c(ctx, l_var3, p)
}

/// Public function `l_msu_ns`.
#[inline(always)]
pub fn l_msu_ns(ctx: &mut DspContext, l_var3: Word32, var1: Word16, var2: Word16) -> Word32 {
    let p = l_mult(ctx, var1, var2);
    l_sub_c(ctx, l_var3, p)
}
