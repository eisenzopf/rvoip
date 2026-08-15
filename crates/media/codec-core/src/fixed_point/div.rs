//! The ETSI `div_s` basic operator.
//!
//! Table-driven transcendentals (`pow2`, `log2`, `inv_sqrt`) deliberately live
//! with their codec rather than here: each codec supplies its own lookup
//! tables, and sharing the code without proving the tables identical would be
//! an assumption dressed as reuse. G.729's live in
//! `codecs::g729::impls::math`.

use crate::fixed_point::shift::{l_shl, shl};
use crate::fixed_point::types::{DspContext, Word16, Word32, MAX_16};

/// Fractional division of two 16-bit values, `var1 / var2` in Q15.
///
/// Returns 0 unless `0 <= var1 <= var2` and `var2 > 0`, which is the domain
/// restriction the ETSI definition imposes on callers.
#[inline(always)]
pub fn div_s(var1: Word16, var2: Word16) -> Word16 {
    if var1.0 < 0 || var2.0 <= 0 || var1.0 > var2.0 {
        return Word16(0);
    }
    if var1.0 == 0 {
        return Word16(0);
    }
    if var1.0 == var2.0 {
        return Word16(MAX_16);
    }

    let mut result = Word16(0);
    let mut l_num = Word32(i32::from(var1.0));
    let l_denom = Word32(i32::from(var2.0));
    let mut ctx = DspContext::default();

    for _ in 0..15 {
        result = shl(&mut ctx, result, 1);
        l_num = l_shl(&mut ctx, l_num, 1);
        if l_num.0 >= l_denom.0 {
            l_num = Word32(l_num.0 - l_denom.0);
            result = Word16(result.0.wrapping_add(1));
        }
    }
    result
}

/// Public function `Div_s`.
#[allow(non_snake_case)]
#[inline(always)]
pub fn Div_s(var1: Word16, var2: Word16) -> Word16 {
    div_s(var1, var2)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn dsp_div_s_half() {
        let r = div_s(Word16(16384), Word16(32767));
        assert!((r.0 - 16384).abs() <= 1);
    }

    #[test]
    fn div_s_domain_restrictions_return_zero() {
        // The ETSI definition is only meaningful for 0 <= var1 <= var2 with
        // var2 > 0; everything else is specified to return 0 rather than
        // trapping, and callers rely on that.
        assert_eq!(div_s(Word16(-1), Word16(100)).0, 0);
        assert_eq!(div_s(Word16(100), Word16(0)).0, 0);
        assert_eq!(div_s(Word16(100), Word16(-5)).0, 0);
        assert_eq!(div_s(Word16(200), Word16(100)).0, 0, "var1 > var2");
        assert_eq!(div_s(Word16(0), Word16(100)).0, 0);
    }

    #[test]
    fn div_s_equal_operands_saturate_to_one() {
        // var1 == var2 is 1.0 in Q15, which saturates to MAX_16.
        assert_eq!(div_s(Word16(1), Word16(1)).0, MAX_16);
        assert_eq!(div_s(Word16(32767), Word16(32767)).0, MAX_16);
    }

    #[test]
    fn div_s_matches_a_rational_model() {
        // Q15 fractional division, truncating: floor(var1 * 32768 / var2),
        // capped at MAX_16.
        for var2 in [1i16, 7, 100, 1000, 12345, 32767] {
            for num in [0i16, 1, 3, 50, 999, 12344, 32766] {
                if num > var2 {
                    continue;
                }
                let got = div_s(Word16(num), Word16(var2)).0;
                let want = if num == var2 {
                    i32::from(MAX_16)
                } else {
                    (i32::from(num) << 15) / i32::from(var2)
                };
                assert_eq!(i32::from(got), want, "div_s({num}, {var2})");
            }
        }
    }
}
