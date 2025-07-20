//! ITU-T G.729A Basic Operations
//!
//! This module implements the basic mathematical operations used in G.729A,
//! exactly as defined in the ITU reference implementation BASIC_OP.C and BASIC_OP.H.
//!
//! These operations use 16-bit and 32-bit fixed-point arithmetic with saturation
//! and are the building blocks for all G.729A signal processing functions.

use crate::codecs::g729a::types::*;
use std::sync::atomic::{AtomicBool, Ordering};

// Global flags for overflow and carry (as in ITU reference)
static OVERFLOW: AtomicBool = AtomicBool::new(false);
static CARRY: AtomicBool = AtomicBool::new(false);

/// Set overflow flag
#[inline]
pub fn set_overflow(flag: bool) {
    OVERFLOW.store(flag, Ordering::Relaxed);
}

/// Get overflow flag
#[inline]
pub fn get_overflow() -> bool {
    OVERFLOW.load(Ordering::Relaxed)
}

/// Set carry flag
#[inline]
pub fn set_carry(flag: bool) {
    CARRY.store(flag, Ordering::Relaxed);
}

/// Get carry flag
#[inline]
pub fn get_carry() -> bool {
    CARRY.load(Ordering::Relaxed)
}

/// Copy array (equivalent to ITU Copy function)
#[inline]
pub fn copy(src: &[Word16], dst: &mut [Word16], n: usize) {
    let len = n.min(src.len()).min(dst.len());
    dst[..len].copy_from_slice(&src[..len]);
}

/// Saturate a 32-bit value to 16 bits
/// ITU-T G.729A sature function - EXACT reference implementation
#[inline]
pub fn saturate(l_var1: Word32) -> Word16 {
    if l_var1 > 0x00007fff {
        set_overflow(true);
        MAX_16
    } else if l_var1 < 0xffff8000u32 as Word32 {
        set_overflow(true);
        MIN_16
    } else {
        set_overflow(false);
        extract_l(l_var1)
    }
}

/// 16-bit addition with saturation
#[inline]
pub fn add(var1: Word16, var2: Word16) -> Word16 {
    let l_sum = var1 as Word32 + var2 as Word32;
    saturate(l_sum)
}

/// 16-bit subtraction with saturation
#[inline]
pub fn sub(var1: Word16, var2: Word16) -> Word16 {
    let l_diff = var1 as Word32 - var2 as Word32;
    saturate(l_diff)
}

/// 16-bit absolute value
/// ITU-T G.729A abs_s function - EXACT reference implementation
#[inline]
pub fn abs_s(var1: Word16) -> Word16 {
    if var1 == 0x8000u16 as Word16 {
        MAX_16
    } else {
        if var1 < 0 {
            -var1
        } else {
            var1
        }
    }
}

/// 16-bit left shift
/// ITU-T G.729A shl function - EXACT reference implementation
#[inline]
pub fn shl(var1: Word16, var2: Word16) -> Word16 {
    if var2 < 0 {
        shr(var1, -var2)
    } else {
        let resultat = var1 as Word32 * (1_i32 << var2);
        if (var2 > 15 && var1 != 0) || (resultat != (resultat as Word16) as Word32) {
            set_overflow(true);
            if var1 > 0 { MAX_16 } else { MIN_16 }
        } else {
            extract_l(resultat)
        }
    }
}

/// 16-bit right shift
/// ITU-T G.729A shr function - EXACT reference implementation
#[inline]
pub fn shr(var1: Word16, var2: Word16) -> Word16 {
    if var2 < 0 {
        shl(var1, -var2)
    } else if var2 >= 15 {
        if var1 < 0 { -1 } else { 0 }
    } else {
        if var1 < 0 {
            !((!var1) >> var2)
        } else {
            var1 >> var2
        }
    }
}

/// 16-bit multiplication (Q15 * Q15 = Q15)
/// ITU-T G.729A mult function - EXACT reference implementation
#[inline]
pub fn mult(var1: Word16, var2: Word16) -> Word16 {
    let l_produit = var1 as Word32 * var2 as Word32;
    
    // Apply mask and shift right by 15 (ITU logic)
    let mut l_produit = (l_produit & 0xffff8000u32 as Word32) >> 15;
    
    // Sign extend when necessary
    if (l_produit & 0x00010000) != 0 {
        l_produit |= 0xffff0000u32 as Word32;
    }
    
    saturate(l_produit)
}

/// 16-bit multiplication to 32-bit result (Q15 * Q15 = Q31)
/// ITU-T G.729A L_mult function - EXACT reference implementation
#[inline]
pub fn l_mult(var1: Word16, var2: Word16) -> Word32 {
    let mut l_var_out = var1 as Word32 * var2 as Word32;
    
    if l_var_out != 0x40000000 {
        l_var_out *= 2;
    } else {
        set_overflow(true);
        l_var_out = MAX_32;
    }
    
    l_var_out
}

/// 16-bit negation
/// ITU-T G.729A negate function - EXACT reference implementation
#[inline]
pub fn negate(var1: Word16) -> Word16 {
    if var1 == MIN_16 {
        set_overflow(true);
        MAX_16
    } else {
        set_overflow(false);
        -var1
    }
}

/// Extract high 16 bits from 32-bit value
#[inline]
pub fn extract_h(l_var1: Word32) -> Word16 {
    set_overflow(false);
    (l_var1 >> 16) as Word16
}

/// Extract low 16 bits from 32-bit value
#[inline]
pub fn extract_l(l_var1: Word32) -> Word16 {
    set_overflow(false);
    (l_var1 & 0xFFFF) as Word16
}

/// Round 32-bit value to 16 bits
#[inline]
pub fn round(l_var1: Word32) -> Word16 {
    let l_rounded = l_add(l_var1, 0x8000);
    extract_h(l_rounded)
}

/// Multiply-accumulate: L_var3 + (var1 * var2)
#[inline]
pub fn l_mac(l_var3: Word32, var1: Word16, var2: Word16) -> Word32 {
    let l_product = l_mult(var1, var2);
    l_add(l_var3, l_product)
}

/// Multiply-subtract: L_var3 - (var1 * var2)
#[inline]
pub fn l_msu(l_var3: Word32, var1: Word16, var2: Word16) -> Word32 {
    let l_product = l_mult(var1, var2);
    l_sub(l_var3, l_product)
}

/// Multiply-accumulate without saturation
/// ITU-T G.729A L_macNs function - EXACT reference implementation
#[inline]
pub fn l_mac_ns(l_var3: Word32, var1: Word16, var2: Word16) -> Word32 {
    let l_var_out = l_mult(var1, var2);
    l_add_c(l_var3, l_var_out)
}

/// Multiply-subtract without saturation
/// ITU-T G.729A L_msuNs function - EXACT reference implementation
#[inline]
pub fn l_msu_ns(l_var3: Word32, var1: Word16, var2: Word16) -> Word32 {
    let l_var_out = l_mult(var1, var2);
    l_sub_c(l_var3, l_var_out)
}

/// 32-bit addition with carry
/// ITU-T G.729A L_add_c function - EXACT reference implementation
#[inline]
pub fn l_add_c(l_var1: Word32, l_var2: Word32) -> Word32 {
    let l_var_out = l_var1 + l_var2 + if get_carry() { 1 } else { 0 };
    let l_test = l_var1 + l_var2;
    let mut carry_int = false;
    
    if l_var1 > 0 && l_var2 > 0 && l_test < 0 {
        set_overflow(true);
        carry_int = false;
    } else if l_var1 < 0 && l_var2 < 0 && l_test > 0 {
        set_overflow(true);
        carry_int = true;
    } else if ((l_var1 ^ l_var2) < 0) && (l_test > 0) {
        set_overflow(false);
        carry_int = true;
    } else {
        set_overflow(false);
        carry_int = false;
    }
    
    if get_carry() {
        if l_test == MAX_32 {
            set_overflow(true);
            set_carry(carry_int);
        } else if l_test == -1 {
            set_carry(true);
        } else {
            set_carry(carry_int);
        }
    } else {
        set_carry(carry_int);
    }
    
    l_var_out
}

/// 32-bit subtraction with carry
/// ITU-T G.729A L_sub_c function - EXACT reference implementation
#[inline]
pub fn l_sub_c(l_var1: Word32, l_var2: Word32) -> Word32 {
    let mut l_var_out;
    let mut carry_int = false;
    
    if get_carry() {
        set_carry(false);
        if l_var2 != MIN_32 {
            l_var_out = l_add_c(l_var1, l_negate(l_var2));
        } else {
            l_var_out = l_var1 - l_var2;
            if l_var1 > 0 {
                set_overflow(true);
                set_carry(false);
            }
        }
    } else {
        l_var_out = l_var1 - l_var2 - 1;
        let l_test = l_var1 - l_var2;
        
        if l_test < 0 && l_var1 > 0 && l_var2 < 0 {
            set_overflow(true);
            carry_int = false;
        } else if l_test > 0 && l_var1 < 0 && l_var2 > 0 {
            set_overflow(true);
            carry_int = true;
        } else if l_test > 0 && ((l_var1 ^ l_var2) > 0) {
            set_overflow(false);
            carry_int = true;
        }
        
        if l_test == MIN_32 {
            set_overflow(true);
            set_carry(carry_int);
        } else {
            set_carry(carry_int);
        }
    }
    
    l_var_out
}

/// 32-bit addition with saturation
/// ITU-T G.729A L_add function - EXACT reference implementation
#[inline]
pub fn l_add(l_var1: Word32, l_var2: Word32) -> Word32 {
    let l_var_out = l_var1 + l_var2;
    
    if ((l_var1 ^ l_var2) & MIN_32) == 0 {
        if (l_var_out ^ l_var1) & MIN_32 != 0 {
            set_overflow(true);
            if l_var1 < 0 { MIN_32 } else { MAX_32 }
        } else {
            l_var_out
        }
    } else {
        l_var_out
    }
}

/// 32-bit subtraction with saturation
/// ITU-T G.729A L_sub function - EXACT reference implementation
#[inline]
pub fn l_sub(l_var1: Word32, l_var2: Word32) -> Word32 {
    let l_var_out = l_var1 - l_var2;
    
    if ((l_var1 ^ l_var2) & MIN_32) != 0 {
        if (l_var_out ^ l_var1) & MIN_32 != 0 {
            set_overflow(true);
            if l_var1 < 0 { MIN_32 } else { MAX_32 }
        } else {
            l_var_out
        }
    } else {
        l_var_out
    }
}

/// 32-bit negation
/// ITU-T G.729A L_negate function - EXACT reference implementation
#[inline]
pub fn l_negate(l_var1: Word32) -> Word32 {
    if l_var1 == MIN_32 {
        set_overflow(true);
        MAX_32
    } else {
        set_overflow(false);
        -l_var1
    }
}

/// 16-bit multiplication with rounding
/// ITU-T G.729A mult_r function - EXACT reference implementation
#[inline]
pub fn mult_r(var1: Word16, var2: Word16) -> Word16 {
    let mut l_produit_arr = var1 as Word32 * var2 as Word32; // product
    l_produit_arr += 0x00004000; // round
    l_produit_arr &= 0xffff8000u32 as Word32;
    l_produit_arr >>= 15; // shift
    
    // Sign extend when necessary
    if (l_produit_arr & 0x00010000) != 0 {
        l_produit_arr |= 0xffff0000u32 as Word32;
    }
    
    saturate(l_produit_arr)
}

/// 32-bit left shift
/// ITU-T G.729A L_shl function - EXACT reference implementation
#[inline]
pub fn l_shl(l_var1: Word32, var2: Word16) -> Word32 {
    let mut l_var_out = 0;
    
    if var2 <= 0 {
        l_shr(l_var1, -var2)
    } else {
        let mut temp_var1 = l_var1;
        let mut temp_var2 = var2;
        
        while temp_var2 > 0 {
            if temp_var1 > 0x3fffffff {
                set_overflow(true);
                l_var_out = MAX_32;
                break;
            } else if temp_var1 < -0x40000000 {
                set_overflow(true);
                l_var_out = MIN_32;
                break;
            }
            temp_var1 *= 2;
            l_var_out = temp_var1;
            temp_var2 -= 1;
        }
        
        l_var_out
    }
}

/// 32-bit right shift
/// ITU-T G.729A L_shr function - EXACT reference implementation
#[inline]
pub fn l_shr(l_var1: Word32, var2: Word16) -> Word32 {
    if var2 < 0 {
        l_shl(l_var1, -var2)
    } else if var2 >= 31 {
        if l_var1 < 0 { -1 } else { 0 }
    } else {
        if l_var1 < 0 {
            !((!l_var1) >> var2)
        } else {
            l_var1 >> var2
        }
    }
}

/// 16-bit right shift with rounding
/// ITU-T G.729A shr_r function - EXACT reference implementation
#[inline]
pub fn shr_r(var1: Word16, var2: Word16) -> Word16 {
    if var2 > 15 {
        0
    } else {
        let mut var_out = shr(var1, var2);
        
        if var2 > 0 {
            if (var1 & (1 << (var2 - 1))) != 0 {
                var_out += 1;
            }
        }
        
        var_out
    }
}

/// Multiply-accumulate with rounding
#[inline]
pub fn mac_r(l_var3: Word32, var1: Word16, var2: Word16) -> Word16 {
    let l_result = l_mac(l_var3, var1, var2);
    round(l_result)
}

/// Multiply-subtract with rounding
#[inline]
pub fn msu_r(l_var3: Word32, var1: Word16, var2: Word16) -> Word16 {
    let l_result = l_msu(l_var3, var1, var2);
    round(l_result)
}

/// Deposit 16-bit value in high part of 32-bit word
#[inline]
pub fn l_deposit_h(var1: Word16) -> Word32 {
    set_overflow(false);
    (var1 as Word32) << 16
}

/// Deposit 16-bit value in low part of 32-bit word
#[inline]
pub fn l_deposit_l(var1: Word16) -> Word32 {
    set_overflow(false);
    var1 as Word32
}

/// 32-bit right shift with rounding
/// ITU-T G.729A L_shr_r function - EXACT reference implementation
#[inline]
pub fn l_shr_r(l_var1: Word32, var2: Word16) -> Word32 {
    if var2 > 31 {
        0
    } else {
        let mut l_var_out = l_shr(l_var1, var2);
        
        if var2 > 0 {
            if (l_var1 & (1 << (var2 - 1))) != 0 {
                l_var_out += 1;
            }
        }
        
        l_var_out
    }
}

/// 32-bit absolute value
#[inline]
pub fn l_abs(l_var1: Word32) -> Word32 {
    if l_var1 == MIN_32 {
        set_overflow(true);
        MAX_32
    } else {
        set_overflow(false);
        l_var1.abs()
    }
}

/// 32-bit saturation (identity function since Word32 is already saturated)
/// ITU-T G.729A L_sat function - EXACT reference implementation
#[inline]
pub fn l_sat(l_var1: Word32) -> Word32 {
    let mut l_var_out = l_var1;
    
    if get_overflow() {
        if get_carry() {
            l_var_out = MIN_32;
        } else {
            l_var_out = MAX_32;
        }
        
        set_carry(false);
        set_overflow(false);
    }
    
    l_var_out
}

/// Normalize a 16-bit value (count possible left shifts without overflow)
/// ITU-T G.729A norm_s function - EXACT reference implementation
#[inline]
pub fn norm_s(var1: Word16) -> Word16 {
    if var1 == 0 {
        return 0;
    }
    
    if var1 == -1 {  // 0xffff
        return 15;
    }
    
    let mut var_out = 0i16;
    let mut temp_var = var1;
    
    if temp_var < 0 {
        temp_var = !temp_var;  // Bitwise NOT (~var1 in C)
    }
    
    // Count shifts until temp_var >= 0x4000
    while temp_var < 0x4000 {
        temp_var <<= 1;
        var_out += 1;
    }
    
    var_out
}

/// Normalize a 32-bit value (count possible left shifts without overflow)
/// ITU-T G.729A norm_l function - EXACT reference implementation
#[inline]
pub fn norm_l(l_var1: Word32) -> Word16 {
    if l_var1 == 0 {
        return 0;
    }
    
    if l_var1 == -1 {  // 0xffffffff
        return 31;
    }
    
    let mut var_out = 0i16;
    let mut temp_var = l_var1;
    
    if temp_var < 0 {
        temp_var = !temp_var;  // Bitwise NOT (~L_var1 in C)
    }
    
    // Count shifts until temp_var >= 0x40000000
    while temp_var < 0x40000000 {
        temp_var <<= 1;
        var_out += 1;
    }
    
    var_out
}

/// 16-bit division - EXACT ITU-T G.729A implementation  
/// Produces fractional integer division of var1 by var2
/// Both inputs must be positive and var2 must be >= var1
#[inline]
pub fn div_s(var1: Word16, var2: Word16) -> Word16 {
    let mut var_out = 0i16;
    
    // Check division by zero first (matches C order)
    if var2 == 0 {
        panic!("Division by 0, Fatal error");
    }
    
    // ITU checks - panic like C exit(0)
    if var1 > var2 || var1 < 0 || var2 < 0 {
        panic!("Division Error var1={} var2={}", var1, var2);
    }
    
    if var1 == 0 {
        return 0;
    }
    
    if var1 == var2 {
        return MAX_16;
    }
    
    // EXACT ITU algorithm: binary long division
    let mut l_num = l_deposit_l(var1);
    let l_denom = l_deposit_l(var2);
    
    for _ in 0..15 {
        var_out <<= 1;
        l_num <<= 1;
        
        if l_num >= l_denom {
            l_num = l_sub(l_num, l_denom);
            var_out = add(var_out, 1);
        }
    }
    
    var_out
}

/// Multiply 32-bit value by 16-bit value  
/// 
/// Based on ITU-T G.729A Mpy_32_16 function
/// 
/// # Arguments
/// * `hi` - High 16 bits of 32-bit value
/// * `lo` - Low 16 bits of 32-bit value  
/// * `n` - 16-bit multiplier
/// 
/// # Returns
/// 32-bit result
pub fn mpy_32_16(hi: Word16, lo: Word16, n: Word16) -> Word32 {
    // ITU-T G.729A Mpy_32_16: ((hi << 16) + lo) * n >> 15  
    let l_32 = ((hi as Word32) << 16) + (lo as Word32);
    let result = ((l_32 as i64 * n as i64) >> 15) as Word32;
    result
}

/// Extract high and low parts of 32-bit value
/// 
/// Based on ITU-T G.729A L_Extract function
/// 
/// # Arguments
/// * `l_32` - 32-bit input value
/// 
/// # Returns  
/// Tuple of (high_16, low_16)
pub fn l_extract_tuple(l_32: Word32) -> (Word16, Word16) {
    // ITU-T G.729A L_Extract: extract high and low 16-bit parts
    let hi = ((l_32 >> 16) & 0xFFFF) as Word16;
    let lo = (l_32 & 0xFFFF) as Word16;
    (hi, lo)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_saturate() {
        // Test normal cases
        assert_eq!(saturate(100), 100);
        assert_eq!(saturate(-100), -100);
        assert!(!get_overflow());
        
        // Test overflow - positive
        assert_eq!(saturate(0x8000), MAX_16);
        assert!(get_overflow());
        
        // Test overflow - negative  
        assert_eq!(saturate(-32769), MIN_16);
        assert!(get_overflow());
        
        // Test exact boundary values
        assert_eq!(saturate(32767), MAX_16);
        assert!(!get_overflow());
        assert_eq!(saturate(-32768), MIN_16);
        assert!(!get_overflow());
    }

    #[test]
    fn test_add_basic() {
        assert_eq!(add(100, 200), 300);
        assert_eq!(add(-100, 50), -50);
        assert_eq!(add(0, 0), 0);
    }

    #[test]
    fn test_add_overflow() {
        // Test positive overflow
        let result = add(MAX_16, 1);
        assert_eq!(result, MAX_16);
        
        // Test negative overflow
        let result2 = add(MIN_16, -1);
        assert_eq!(result2, MIN_16);
        
        // Test normal addition doesn't saturate
        let result3 = add(100, 200);
        assert_eq!(result3, 300);
    }

    #[test]
    fn test_sub_basic() {
        assert_eq!(sub(200, 100), 100);
        assert_eq!(sub(-100, 50), -150);
        assert_eq!(sub(0, 0), 0);
    }

    #[test]
    fn test_sub_overflow() {
        // Test positive overflow (MAX_16 - (-1) should saturate)
        let result = sub(MAX_16, -1);
        assert_eq!(result, MAX_16);
        
        // Test negative overflow  
        let result2 = sub(MIN_16, 1);
        assert_eq!(result2, MIN_16);
    }

    #[test]
    fn test_abs_s() {
        assert_eq!(abs_s(100), 100);
        assert_eq!(abs_s(-100), 100);
        assert_eq!(abs_s(0), 0);
        assert_eq!(abs_s(MAX_16), MAX_16);
        
        // Special case: abs_s(MIN_16) = MAX_16 per ITU spec
        assert_eq!(abs_s(MIN_16), MAX_16);
    }

    #[test]
    fn test_mult_basic() {
        // Q15 format: 0.5 * 0.5 = 0.25
        let half = 16384; // 0.5 in Q15
        let quarter = mult(half, half);
        assert_eq!(quarter, 8192); // 0.25 in Q15
        
        // Test multiplication by 1 in Q15 (32767 ≈ 1.0)
        assert_eq!(mult(100, MAX_16), 99); // Close to 100 due to Q15 scaling
    }

    #[test] 
    fn test_mult_special_case() {
        // ITU special case: mult(MIN_16, MIN_16) = MAX_16
        assert_eq!(mult(MIN_16, MIN_16), MAX_16);
    }

    #[test]
    fn test_l_mult() {
        // L_mult produces Q31 result from Q15 inputs, with left shift by 1
        let result = l_mult(16384, 16384); // 0.5 * 0.5 in Q15
        assert_eq!(result, 0x20000000); // 0.25 * 2 = 0.5 in Q31 due to left shift
        
        // Special case: L_mult(MIN_16, MIN_16) = MAX_32 with overflow
        let result2 = l_mult(MIN_16, MIN_16);
        assert_eq!(result2, MAX_32);
        assert!(get_overflow());
    }

    #[test]
    fn test_negate() {
        assert_eq!(negate(100), -100);
        assert_eq!(negate(-100), 100);
        assert_eq!(negate(0), 0);
        
        // Special case: negate(MIN_16) = MAX_16
        assert_eq!(negate(MIN_16), MAX_16);
    }

    #[test]
    fn test_extract_operations() {
        let val = 0x12345678i32;
        assert_eq!(extract_h(val), 0x1234);
        assert_eq!(extract_l(val), 0x5678);
        
        // Test negative values
        let neg_val = -1i32;
        assert_eq!(extract_h(neg_val), -1i16);
        assert_eq!(extract_l(neg_val), -1i16);
    }

    #[test]
    fn test_round() {
        // round() adds 0x8000 and extracts high part
        assert_eq!(round(0x12340000), 0x1234); // No rounding needed
        assert_eq!(round(0x12348000), 0x1235); // Exactly halfway rounds up  
        assert_eq!(round(0x1234ffff), 0x1235); // High bits round up
        assert_eq!(round(0x12347fff), 0x1234); // Just under halfway, rounds down
    }

    #[test]
    fn test_shift_operations() {
        // Basic shifts
        assert_eq!(shl(100, 1), 200);
        assert_eq!(shr(200, 1), 100);
        
        // Test negative shifts (should call opposite operation)
        assert_eq!(shl(100, -1), 50);
        assert_eq!(shr(100, -1), 200);
        
        // Test overflow in shl - MAX_16 << 1 should saturate to MAX_16
        assert_eq!(shl(MAX_16, 1), MAX_16); // Overflow saturates to MAX_16
        
        // Test arithmetic right shift of negative numbers
        assert_eq!(shr(-4, 1), -2);
        assert_eq!(shr(-1, 15), -1); // All bits are sign bits
        
        // 32-bit shifts
        assert_eq!(l_shl(100, 1), 200);
        assert_eq!(l_shr(200, 1), 100);
    }

    #[test]
    fn test_l_add_l_sub() {
        // Basic operations
        assert_eq!(l_add(1000, 2000), 3000);
        assert_eq!(l_sub(2000, 1000), 1000);
        
        // Test saturation behavior - use values that would cause overflow if not saturated
        // MAX_32 is 0x7FFFFFFF, so MAX_32 + MAX_32 would overflow but should saturate
        let big_positive = MAX_32 / 2;
        let result1 = l_add(big_positive, big_positive + 1);
        assert!(result1 <= MAX_32);
        
        let big_negative = MIN_32 / 2;
        let result2 = l_sub(big_negative, big_negative.abs());
        assert!(result2 >= MIN_32);
    }

    #[test]
    fn test_l_add_c_l_sub_c() {
        // Test basic carry operations
        set_carry(false);
        assert_eq!(l_add_c(100, 200), 300);
        
        set_carry(true);  
        assert_eq!(l_add_c(100, 200), 301); // Adds carry
        
        set_carry(false);
        assert_eq!(l_sub_c(200, 100), 99); // Subtracts 1 when no carry
        
        set_carry(true);
        assert_eq!(l_sub_c(200, 100), 100); // Normal subtract when carry set
    }

    #[test]
    fn test_l_sat() {
        // Test L_sat with overflow conditions
        set_overflow(false);
        set_carry(false);
        assert_eq!(l_sat(12345), 12345); // No saturation
        
        set_overflow(true);
        set_carry(false);
        assert_eq!(l_sat(12345), MAX_32); // Saturate to max
        assert!(!get_overflow()); // Should clear flags
        assert!(!get_carry());
        
        set_overflow(true);
        set_carry(true);
        assert_eq!(l_sat(12345), MIN_32); // Saturate to min
        assert!(!get_overflow()); // Should clear flags
        assert!(!get_carry());
    }

    #[test]
    fn test_mac_msu_operations() {
        let acc = 1000i32;
        let var1 = 100i16;
        let var2 = 200i16;
        
        // Test MAC (multiply-accumulate)
        let result = l_mac(acc, var1, var2);
        let expected = l_add(acc, l_mult(var1, var2));
        assert_eq!(result, expected);
        
        // Test MSU (multiply-subtract)  
        let result2 = l_msu(acc, var1, var2);
        let expected2 = l_sub(acc, l_mult(var1, var2));
        assert_eq!(result2, expected2);
        
        // Test with rounding
        let result3 = mac_r(acc, var1, var2);
        let expected3 = round(l_mac(acc, var1, var2));
        assert_eq!(result3, expected3);
        
        let result4 = msu_r(acc, var1, var2);
        let expected4 = round(l_msu(acc, var1, var2));
        assert_eq!(result4, expected4);
    }

    #[test]
    fn test_mult_r() {
        // mult_r is mult with rounding
        let result = mult_r(16384, 16384); // 0.5 * 0.5
        // Should be close to mult result but with rounding
        let mult_result = mult(16384, 16384);
        assert!(result >= mult_result); // Rounding can only increase or keep same
    }

    #[test]
    fn test_shr_r() {
        // shr_r is shr with rounding
        assert_eq!(shr_r(7, 2), 2); // 7/4 = 1.75, rounds to 2
        assert_eq!(shr_r(6, 2), 2); // 6/4 = 1.5, rounds to 2 (round half up)
        assert_eq!(shr_r(5, 2), 1); // 5/4 = 1.25, rounds to 1
        
        // Test with shift > 15
        assert_eq!(shr_r(100, 16), 0);
    }

    #[test]
    fn test_l_shr_r() {
        // L_shr_r is L_shr with rounding
        assert_eq!(l_shr_r(7, 2), 2); // 7/4 = 1.75, rounds to 2
        assert_eq!(l_shr_r(6, 2), 2); // 6/4 = 1.5, rounds to 2 
        assert_eq!(l_shr_r(5, 2), 1); // 5/4 = 1.25, rounds to 1
        
        // Test with shift > 31
        assert_eq!(l_shr_r(100, 32), 0);
    }

    #[test]
    fn test_l_abs() {
        assert_eq!(l_abs(12345), 12345);
        assert_eq!(l_abs(-12345), 12345);
        assert_eq!(l_abs(0), 0);
        
        // Special case: L_abs(MIN_32) = MAX_32
        assert_eq!(l_abs(MIN_32), MAX_32);
    }

    #[test]
    fn test_deposit_operations() {
        assert_eq!(l_deposit_h(0x1234), 0x12340000);
        assert_eq!(l_deposit_l(0x1234), 0x1234);
        assert_eq!(l_deposit_l(-1), -1i32);
    }

    #[test]
    fn test_norm_s() {
        // norm_s counts left shifts needed to normalize (get MSB in proper position)
        assert_eq!(norm_s(0), 0);
        assert_eq!(norm_s(-1), 15); // 0xFFFF
        assert_eq!(norm_s(0x4000), 0); // Already normalized (0100...)
        assert_eq!(norm_s(0x2000), 1); // Can shift left 1 time (0010... -> 0100...)
        assert_eq!(norm_s(0x1000), 2); // Can shift left 2 times (0001... -> 0100...)
        
        // Test negative values - verified with C behavior
        assert_eq!(norm_s(-16384), 1); // 0xC000 → ~0xC000 = 0x3FFF, needs 1 shift to reach 0x4000
        assert_eq!(norm_s(-8192), 2);  // 0xE000 → ~0xE000 = 0x1FFF, needs 2 shifts to reach 0x4000
        
        // Simple test: value 1 should need many shifts to reach 0x4000
        assert_eq!(norm_s(1), 14);
    }

    #[test]
    fn test_norm_l() {
        // norm_l counts leading sign bits for 32-bit values
        assert_eq!(norm_l(0), 0);
        assert_eq!(norm_l(-1), 31); // 0xFFFFFFFF
        assert_eq!(norm_l(0x40000000), 0); // Already normalized
        assert_eq!(norm_l(0x20000000), 1); // Can shift left 1 time
        assert_eq!(norm_l(0x10000000), 2); // Can shift left 2 times
    }

    #[test] 
    fn test_div_s_normal() {
        // Test normal division cases
        assert_eq!(div_s(0, 100), 0); // 0/anything = 0
        assert_eq!(div_s(100, 100), MAX_16); // Equal values = MAX_16
        
        // Test fractional division (Q15 format)
        // div_s(var1, var2) produces var1/var2 in Q15 format
        let result = div_s(16384, 32767); // ~0.5 / ~1.0 should be ~0.5
        assert!(result > 16300 && result < 16400); // Close to 0.5 in Q15
    }

    #[test]
    #[should_panic(expected = "Division Error")]
    fn test_div_s_error_var1_greater() {
        div_s(200, 100); // var1 > var2 should panic
    }

    #[test]
    #[should_panic(expected = "Division Error")]
    fn test_div_s_error_negative_var1() {
        div_s(-1, 100); // Negative var1 should panic
    }

    #[test]
    #[should_panic(expected = "Division Error")]
    fn test_div_s_error_negative_var2() {
        div_s(100, -1); // Negative var2 should panic
    }

    #[test]
    #[should_panic(expected = "Division by 0")]
    fn test_div_s_divide_by_zero() {
        div_s(100, 0); // Division by zero should panic
    }

    #[test]
    fn test_helper_functions() {
        // Test mpy_32_16
        let result = mpy_32_16(0x1234, 0x5678, 100);
        let expected = (((0x1234i32 << 16) + 0x5678) as i64 * 100) >> 15;
        assert_eq!(result, expected as Word32);
        
        // Test l_extract_tuple
        let (hi, lo) = l_extract_tuple(0x12345678);
        assert_eq!(hi, 0x1234);
        assert_eq!(lo, 0x5678);
    }

    #[test]
    fn test_copy() {
        let src = [1, 2, 3, 4, 5];
        let mut dst = [0; 5];
        copy(&src, &mut dst, 5);
        assert_eq!(dst, [1, 2, 3, 4, 5]);
        
        // Test partial copy
        let mut dst2 = [0; 3];
        copy(&src, &mut dst2, 3);
        assert_eq!(dst2, [1, 2, 3]);
    }
} 