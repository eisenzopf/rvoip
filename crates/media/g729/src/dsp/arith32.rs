#![allow(clippy::collapsible_if)]

use crate::dsp::arith::sature;
use crate::dsp::types::{DspContext, MAX_32, MIN_32, Word16, Word32};

#[path = "arith32_ext.rs"]
mod arith32_ext;
#[path = "arith32_compat.rs"]
mod compat;
/// Public re-export.
pub use arith32_ext::{l_add_c, l_mac_ns, l_msu_ns, l_sub_c};
/// Public re-export.
pub use compat::{L_add, L_mac, L_msu, L_mult, L_sub, l_abs, l_sat, mac_r, msu_r};

/// Public function `l_mult`.
#[inline(always)]
pub fn l_mult(ctx: &mut DspContext, var1: Word16, var2: Word16) -> Word32 {
    let mut out = i32::from(var1.0).wrapping_mul(i32::from(var2.0));
    if out != 0x4000_0000 {
        out <<= 1;
    } else {
        ctx.overflow = true;
        out = MAX_32;
    }
    Word32(out)
}

/// Public function `l_add`.
#[inline(always)]
pub fn l_add(ctx: &mut DspContext, l_var1: Word32, l_var2: Word32) -> Word32 {
    let x = l_var1.0;
    let y = l_var2.0;
    let mut out = x.wrapping_add(y);

    if ((x ^ y) & MIN_32) == 0 {
        if ((out ^ x) & MIN_32) != 0 {
            out = if x < 0 { MIN_32 } else { MAX_32 };
            ctx.overflow = true;
        }
    }

    Word32(out)
}

/// Public function `l_sub`.
#[inline(always)]
pub fn l_sub(ctx: &mut DspContext, l_var1: Word32, l_var2: Word32) -> Word32 {
    let x = l_var1.0;
    let y = l_var2.0;
    let mut out = x.wrapping_sub(y);

    if ((x ^ y) & MIN_32) != 0 {
        if ((out ^ x) & MIN_32) != 0 {
            out = if x < 0 { MIN_32 } else { MAX_32 };
            ctx.overflow = true;
        }
    }

    Word32(out)
}

/// Public function `l_mac`.
#[inline(always)]
pub fn l_mac(ctx: &mut DspContext, l_var3: Word32, var1: Word16, var2: Word16) -> Word32 {
    let p = l_mult(ctx, var1, var2);
    l_add(ctx, l_var3, p)
}

/// Public function `l_msu`.
#[inline(always)]
pub fn l_msu(ctx: &mut DspContext, l_var3: Word32, var1: Word16, var2: Word16) -> Word32 {
    let p = l_mult(ctx, var1, var2);
    l_sub(ctx, l_var3, p)
}

/// Public function `l_negate`.
#[inline(always)]
pub fn l_negate(_ctx: &mut DspContext, l_var1: Word32) -> Word32 {
    if l_var1.0 == MIN_32 {
        Word32(MAX_32)
    } else {
        Word32(-l_var1.0)
    }
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

/// Public constant `fn`.
#[inline(always)]
pub const fn l_deposit_h(var1: Word16) -> Word32 {
    Word32((var1.0 as i32) << 16)
}

/// Public constant `fn`.
#[inline(always)]
pub const fn l_deposit_l(var1: Word16) -> Word32 {
    Word32(var1.0 as i32)
}
