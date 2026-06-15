use crate::dsp::arith::{add, extract_l};
use crate::dsp::types::{DspContext, MAX_16, MAX_32, MIN_16, MIN_32, Word16, Word32};

/// Public function `shl`.
#[inline(always)]
pub fn shl(ctx: &mut DspContext, var1: Word16, var2: i16) -> Word16 {
    if var2 < 0 {
        return shr(ctx, var1, -var2);
    }

    let resultat = i64::from(var1.0) * (1i64 << var2);
    if (var2 > 15 && var1.0 != 0) || resultat != i64::from(resultat as i16) {
        ctx.overflow = true;
        Word16(if var1.0 > 0 { MAX_16 } else { MIN_16 })
    } else {
        extract_l(Word32(resultat as i32))
    }
}

/// Public function `shr`.
#[inline(always)]
pub fn shr(ctx: &mut DspContext, var1: Word16, var2: i16) -> Word16 {
    if var2 < 0 {
        return shl(ctx, var1, -var2);
    }

    if var2 >= 15 {
        return Word16(if var1.0 < 0 { -1 } else { 0 });
    }

    if var1.0 < 0 {
        Word16(!((!var1.0) >> var2))
    } else {
        Word16(var1.0 >> var2)
    }
}

/// Public function `l_shl`.
#[inline(always)]
pub fn l_shl(ctx: &mut DspContext, l_var1: Word32, var2: i16) -> Word32 {
    if var2 <= 0 {
        return l_shr(ctx, l_var1, -var2);
    }

    let mut acc = l_var1.0;
    let mut out = 0i32;
    let mut n = var2;

    while n > 0 {
        if acc > 0x3FFF_FFFF {
            ctx.overflow = true;
            out = MAX_32;
            break;
        } else if acc < 0xC000_0000u32 as i32 {
            ctx.overflow = true;
            out = MIN_32;
            break;
        }
        acc = acc.wrapping_mul(2);
        out = acc;
        n -= 1;
    }

    Word32(out)
}

/// Public function `l_shr`.
#[inline(always)]
pub fn l_shr(ctx: &mut DspContext, l_var1: Word32, var2: i16) -> Word32 {
    if var2 < 0 {
        return l_shl(ctx, l_var1, -var2);
    }

    if var2 >= 31 {
        return Word32(if l_var1.0 < 0 { -1 } else { 0 });
    }

    if l_var1.0 < 0 {
        Word32(!((!l_var1.0) >> var2))
    } else {
        Word32(l_var1.0 >> var2)
    }
}

/// Public function `shr_r`.
#[inline(always)]
pub fn shr_r(ctx: &mut DspContext, var1: Word16, var2: i16) -> Word16 {
    if var2 > 15 {
        return Word16(0);
    }

    let mut out = shr(ctx, var1, var2);
    if var2 > 0 && (var1.0 & ((1i16) << (var2 - 1))) != 0 {
        out = add(ctx, out, Word16(1));
    }
    out
}

/// Public function `l_shr_r`.
#[inline(always)]
pub fn l_shr_r(ctx: &mut DspContext, l_var1: Word32, var2: i16) -> Word32 {
    if var2 > 31 {
        return Word32(0);
    }

    let mut out = l_shr(ctx, l_var1, var2);
    if var2 > 0 && (l_var1.0 & (1i32 << (var2 - 1))) != 0 {
        out = Word32(out.0.wrapping_add(1));
    }
    out
}

/// Public function `norm_s`.
#[inline(always)]
pub fn norm_s(var1: Word16) -> i16 {
    if var1.0 == 0 {
        return 0;
    }
    if var1.0 == -1 {
        return 15;
    }

    let mut x = var1.0;
    if x < 0 {
        x = !x;
    }

    let mut out = 0;
    while x < 0x4000 {
        x <<= 1;
        out += 1;
    }
    out
}

/// Public function `norm_l`.
#[inline(always)]
pub fn norm_l(l_var1: Word32) -> i16 {
    if l_var1.0 == 0 {
        return 0;
    }
    if l_var1.0 == -1 {
        return 31;
    }

    let mut x = l_var1.0;
    if x < 0 {
        x = !x;
    }

    let mut out = 0;
    while x < 0x4000_0000 {
        x <<= 1;
        out += 1;
    }
    out
}

/// Public function `L_shl`.
#[allow(non_snake_case)]
#[inline(always)]
pub fn L_shl(ctx: &mut DspContext, l_var1: Word32, var2: i16) -> Word32 {
    l_shl(ctx, l_var1, var2)
}

/// Public function `L_shr`.
#[allow(non_snake_case)]
#[inline(always)]
pub fn L_shr(ctx: &mut DspContext, l_var1: Word32, var2: i16) -> Word32 {
    l_shr(ctx, l_var1, var2)
}

/// Public function `L_shr_r`.
#[allow(non_snake_case)]
#[inline(always)]
pub fn L_shr_r(ctx: &mut DspContext, l_var1: Word32, var2: i16) -> Word32 {
    l_shr_r(ctx, l_var1, var2)
}

#[cfg(test)]
#[path = "shift_tests.rs"]
mod shift_tests;
