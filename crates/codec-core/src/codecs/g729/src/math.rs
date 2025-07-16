//! Fixed-Point Arithmetic Operations for G.729
//!
//! This module implements the basic fixed-point arithmetic operations used in G.729,
//! based on the ITU-T reference implementation in BASIC_OP.C.
//!
//! All operations include proper overflow handling and saturation.

use super::types::{Word16, Word32, Flag};

/// Maximum 16-bit value
pub const MAX_16: Word16 = 0x7fff;

/// Minimum 16-bit value  
pub const MIN_16: Word16 = -0x8000;

/// Maximum 32-bit value
pub const MAX_32: Word32 = 0x7fffffff;

/// Minimum 32-bit value
pub const MIN_32: Word32 = -0x80000000;

/// Global overflow flag (mimics ITU implementation)
thread_local! {
    static OVERFLOW: std::cell::Cell<Flag> = std::cell::Cell::new(0);
    static CARRY: std::cell::Cell<Flag> = std::cell::Cell::new(0);
}

/// Get overflow flag
pub fn get_overflow() -> Flag {
    OVERFLOW.with(|f| f.get())
}

/// Set overflow flag
pub fn set_overflow(value: Flag) {
    OVERFLOW.with(|f| f.set(value));
}

/// Get carry flag
pub fn get_carry() -> Flag {
    CARRY.with(|f| f.get())
}

/// Set carry flag  
pub fn set_carry(value: Flag) {
    CARRY.with(|f| f.set(value));
}

/// Limit 32-bit value to 16-bit range with saturation
pub fn sature(l_var1: Word32) -> Word16 {
    if l_var1 > 0x00007fff {
        set_overflow(1);
        MAX_16
    } else if l_var1 < -0x00008000 {
        set_overflow(1);
        MIN_16
    } else {
        set_overflow(0);
        extract_l(l_var1)
    }
}

/// 16-bit addition with saturation
pub fn add(var1: Word16, var2: Word16) -> Word16 {
    let l_sum = var1 as Word32 + var2 as Word32;
    sature(l_sum)
}

/// 16-bit subtraction with saturation
pub fn sub(var1: Word16, var2: Word16) -> Word16 {
    let l_diff = var1 as Word32 - var2 as Word32;
    sature(l_diff)
}

/// 16-bit absolute value with saturation
pub fn abs_s(var1: Word16) -> Word16 {
    if var1 == MIN_16 {
        set_overflow(1);
        MAX_16
    } else {
        set_overflow(0);
        var1.abs()
    }
}

/// 16-bit left shift
pub fn shl(var1: Word16, var2: Word16) -> Word16 {
    if var2 < 0 {
        return shr(var1, -var2);
    }
    
    if var2 >= 15 {
        set_overflow(if var1 != 0 { 1 } else { 0 });
        return if var1 > 0 { MAX_16 } else { MIN_16 };
    }
    
    let result = (var1 as Word32) << var2;
    sature(result)
}

/// 16-bit right shift
pub fn shr(var1: Word16, var2: Word16) -> Word16 {
    if var2 < 0 {
        return shl(var1, -var2);
    }
    
    if var2 >= 15 {
        return if var1 < 0 { -1 } else { 0 };
    }
    
    set_overflow(0);
    var1 >> var2
}

/// 16-bit multiplication (Q15 * Q15 = Q15)
pub fn mult(var1: Word16, var2: Word16) -> Word16 {
    if var1 == MIN_16 && var2 == MIN_16 {
        set_overflow(1);
        return MAX_16;
    }
    
    set_overflow(0);
    let l_product = (var1 as Word32 * var2 as Word32) << 1;
    extract_h(l_product)
}

/// 16-bit multiplication to 32-bit result (Q15 * Q15 = Q31)
pub fn l_mult(var1: Word16, var2: Word16) -> Word32 {
    if var1 == MIN_16 && var2 == MIN_16 {
        set_overflow(1);
        return MAX_32;
    }
    
    set_overflow(0);
    (var1 as Word32 * var2 as Word32) << 1
}

/// 16-bit negation
pub fn negate(var1: Word16) -> Word16 {
    if var1 == MIN_16 {
        set_overflow(1);
        MAX_16
    } else {
        set_overflow(0);
        -var1
    }
}

/// Extract high 16 bits from 32-bit value
pub fn extract_h(l_var1: Word32) -> Word16 {
    set_overflow(0);
    (l_var1 >> 16) as Word16
}

/// Extract low 16 bits from 32-bit value
pub fn extract_l(l_var1: Word32) -> Word16 {
    set_overflow(0);
    (l_var1 & 0xffff) as Word16
}

/// Round 32-bit value to 16-bit
pub fn round(l_var1: Word32) -> Word16 {
    let l_rounded = l_add(l_var1, 0x00008000);
    extract_h(l_rounded)
}

/// Multiply-accumulate: L_var3 + (var1 * var2)
pub fn l_mac(l_var3: Word32, var1: Word16, var2: Word16) -> Word32 {
    let l_product = l_mult(var1, var2);
    l_add(l_var3, l_product)
}

/// Multiply-subtract: L_var3 - (var1 * var2)
pub fn l_msu(l_var3: Word32, var1: Word16, var2: Word16) -> Word32 {
    let l_product = l_mult(var1, var2);
    l_sub(l_var3, l_product)
}

/// 32-bit addition with saturation
pub fn l_add(l_var1: Word32, l_var2: Word32) -> Word32 {
    let result = l_var1.saturating_add(l_var2);
    set_overflow(if result == l_var1 + l_var2 { 0 } else { 1 });
    result
}

/// 32-bit subtraction with saturation
pub fn l_sub(l_var1: Word32, l_var2: Word32) -> Word32 {
    let result = l_var1.saturating_sub(l_var2);
    set_overflow(if result == l_var1 - l_var2 { 0 } else { 1 });
    result
}

/// 32-bit negation
pub fn l_negate(l_var1: Word32) -> Word32 {
    if l_var1 == MIN_32 {
        set_overflow(1);
        MAX_32
    } else {
        set_overflow(0);
        -l_var1
    }
}

/// 16-bit multiplication with rounding
pub fn mult_r(var1: Word16, var2: Word16) -> Word16 {
    let l_product = l_mult(var1, var2);
    round(l_product)
}

/// 32-bit left shift
pub fn l_shl(l_var1: Word32, var2: Word16) -> Word32 {
    if var2 <= 0 {
        return l_shr(l_var1, -var2);
    }
    
    if var2 >= 31 {
        set_overflow(if l_var1 != 0 { 1 } else { 0 });
        return if l_var1 > 0 { MAX_32 } else { MIN_32 };
    }
    
    let result = l_var1.checked_shl(var2 as u32).unwrap_or_else(|| {
        set_overflow(1);
        if l_var1 > 0 { MAX_32 } else { MIN_32 }
    });
    
    // Check for overflow
    if result >> var2 != l_var1 {
        set_overflow(1);
        if l_var1 > 0 { MAX_32 } else { MIN_32 }
    } else {
        set_overflow(0);
        result
    }
}

/// 32-bit right shift
pub fn l_shr(l_var1: Word32, var2: Word16) -> Word32 {
    if var2 < 0 {
        return l_shl(l_var1, -var2);
    }
    
    if var2 >= 31 {
        return if l_var1 < 0 { -1 } else { 0 };
    }
    
    set_overflow(0);
    l_var1 >> var2
}

/// 16-bit right shift with rounding
pub fn shr_r(var1: Word16, var2: Word16) -> Word16 {
    if var2 >= 15 {
        return 0;
    }
    
    if var2 <= 0 {
        return var1;
    }
    
    let l_var = (var1 as Word32) + (1 << (var2 - 1));
    extract_h(l_shl(l_var, 16 - var2))
}

/// Multiply-accumulate with rounding
pub fn mac_r(l_var3: Word32, var1: Word16, var2: Word16) -> Word16 {
    let l_result = l_mac(l_var3, var1, var2);
    round(l_result)
}

/// Multiply-subtract with rounding
pub fn msu_r(l_var3: Word32, var1: Word16, var2: Word16) -> Word16 {
    let l_result = l_msu(l_var3, var1, var2);
    round(l_result)
}

/// Deposit 16-bit value in high part of 32-bit word
pub fn l_deposit_h(var1: Word16) -> Word32 {
    set_overflow(0);
    (var1 as Word32) << 16
}

/// Deposit 16-bit value in low part of 32-bit word
pub fn l_deposit_l(var1: Word16) -> Word32 {
    set_overflow(0);
    var1 as Word32 & 0xffff
}

/// 32-bit right shift with rounding
pub fn l_shr_r(l_var1: Word32, var2: Word16) -> Word32 {
    if var2 <= 0 {
        return l_var1;
    }
    
    if var2 >= 31 {
        return if l_var1 < 0 { -1 } else { 0 };
    }
    
    let l_result = l_add(l_var1, 1 << (var2 - 1));
    l_shr(l_result, var2)
}

/// 32-bit absolute value
pub fn l_abs(l_var1: Word32) -> Word32 {
    if l_var1 == MIN_32 {
        set_overflow(1);
        MAX_32
    } else {
        set_overflow(0);
        l_var1.abs()
    }
}

/// 32-bit saturation
pub fn l_sat(l_var1: Word32) -> Word32 {
    set_overflow(0);
    l_var1.clamp(MIN_32, MAX_32)
}

/// Count leading zeros in 16-bit value (normalization)
pub fn norm_s(var1: Word16) -> Word16 {
    if var1 == 0 {
        return 0;
    }
    
    let abs_val = abs_s(var1) as u16;
    (abs_val.leading_zeros() as Word16) - 1
}

/// Count leading zeros in 32-bit value (normalization)
pub fn norm_l(l_var1: Word32) -> Word16 {
    if l_var1 == 0 {
        return 0;
    }
    
    let abs_val = l_abs(l_var1) as u32;
    (abs_val.leading_zeros() as Word16) - 1
}

/// 16-bit division
pub fn div_s(var1: Word16, var2: Word16) -> Word16 {
    if var2 == 0 {
        set_overflow(1);
        return if var1 >= 0 { MAX_16 } else { MIN_16 };
    }
    
    if var1 == 0 {
        set_overflow(0);
        return 0;
    }
    
    if abs_s(var1) >= abs_s(var2) {
        set_overflow(1);
        return if (var1 > 0) == (var2 > 0) { MAX_16 } else { MIN_16 };
    }
    
    set_overflow(0);
    let l_num = l_deposit_h(var1);
    let l_denom = l_deposit_h(var2);
    
    // Perform long division
    let mut l_quotient = 0;
    let mut l_remainder = l_abs(l_num);
    let l_divisor = l_abs(l_denom);
    
    for _ in 0..15 {
        l_remainder = l_shl(l_remainder, 1);
        l_quotient = l_shl(l_quotient, 1);
        
        if l_remainder >= l_divisor {
            l_remainder = l_sub(l_remainder, l_divisor);
            l_quotient = l_add(l_quotient, 1);
        }
    }
    
    let result = extract_l(l_quotient);
    if (var1 > 0) != (var2 > 0) {
        negate(result)
    } else {
        result
    }
}

/// Set array of Word16 to zero
pub fn set_zero_16(array: &mut [Word16]) {
    array.fill(0);
}

/// Set array of Word32 to zero
pub fn set_zero_32(array: &mut [Word32]) {
    array.fill(0);
}

/// Copy array of Word16
pub fn copy_16(src: &[Word16], dst: &mut [Word16]) {
    let len = src.len().min(dst.len());
    dst[..len].copy_from_slice(&src[..len]);
}

/// Copy array of Word32
pub fn copy_32(src: &[Word32], dst: &mut [Word32]) {
    let len = src.len().min(dst.len());
    dst[..len].copy_from_slice(&src[..len]);
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_add_basic() {
        assert_eq!(add(100, 200), 300);
        assert_eq!(add(-100, 50), -50);
    }

    #[test]
    fn test_add_overflow() {
        let result = add(MAX_16, 1);
        assert_eq!(result, MAX_16);
        assert_eq!(get_overflow(), 1);
    }

    #[test]
    fn test_sub_basic() {
        assert_eq!(sub(300, 200), 100);
        assert_eq!(sub(50, 100), -50);
    }

    #[test]
    fn test_mult_basic() {
        // Q15 format: 0.5 * 0.5 = 0.25
        let half = 16384; // 0.5 in Q15
        let quarter = 8192; // 0.25 in Q15
        assert_eq!(mult(half, half), quarter);
    }

    #[test]
    fn test_l_mult_basic() {
        let result = l_mult(16384, 16384); // 0.5 * 0.5 in Q15
        assert_eq!(result, 536870912); // 0.25 in Q31
    }

    #[test]
    fn test_abs_s() {
        assert_eq!(abs_s(100), 100);
        assert_eq!(abs_s(-100), 100);
        
        // Test saturation case
        let result = abs_s(MIN_16);
        assert_eq!(result, MAX_16);
        assert_eq!(get_overflow(), 1);
    }

    #[test]
    fn test_shl_shr() {
        assert_eq!(shl(100, 1), 200);
        assert_eq!(shr(200, 1), 100);
        
        // Test negative shift
        assert_eq!(shl(100, -1), 50);
        assert_eq!(shr(100, -1), 200);
    }

    #[test]
    fn test_extract() {
        let val = 0x12345678;
        assert_eq!(extract_h(val), 0x1234);
        assert_eq!(extract_l(val), 0x5678);
    }

    #[test]
    fn test_norm_s() {
        assert_eq!(norm_s(0), 0);
        assert_eq!(norm_s(1), 14);
        assert_eq!(norm_s(MAX_16), 0);
    }
} 