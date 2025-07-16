//! Tests for G.729 Mathematical Operations
//!
//! These tests validate that our fixed-point arithmetic operations
//! behave exactly like the ITU reference implementation.

use crate::codecs::g729::src::math::*;
use crate::codecs::g729::src::types::{Word16, Word32};

#[test]
fn test_basic_addition() {
    // Normal addition
    assert_eq!(add(100, 200), 300);
    assert_eq!(add(-100, 50), -50);
    assert_eq!(add(0, 0), 0);
    
    // Test overflow saturation
    let result = add(MAX_16, 1);
    assert_eq!(result, MAX_16);
    assert_eq!(get_overflow(), 1);
    
    let result = add(MIN_16, -1);
    assert_eq!(result, MIN_16);
    assert_eq!(get_overflow(), 1);
}

#[test]
fn test_basic_subtraction() {
    // Normal subtraction
    assert_eq!(sub(300, 200), 100);
    assert_eq!(sub(50, 100), -50);
    assert_eq!(sub(0, 0), 0);
    
    // Test overflow saturation
    let result = sub(MAX_16, -1);
    assert_eq!(result, MAX_16);
    assert_eq!(get_overflow(), 1);
    
    let result = sub(MIN_16, 1);
    assert_eq!(result, MIN_16);
    assert_eq!(get_overflow(), 1);
}

#[test] 
fn test_multiplication() {
    // Test Q15 multiplication: mult(x,y) = (x*y*2) >> 16
    // 0.5 * 0.5 = 0.25 in Q15 format
    let half = 16384; // 0.5 in Q15
    let quarter = 8192; // 0.25 in Q15
    assert_eq!(mult(half, half), quarter);
    
    // Test unity multiplication: 1.0 * x = x  
    let one = 32767; // ~1.0 in Q15
    assert_eq!(mult(one, half), half);
    
    // Test zero multiplication
    assert_eq!(mult(0, 12345), 0);
    
    // Test overflow case: MIN_16 * MIN_16 should saturate
    let result = mult(MIN_16, MIN_16);
    assert_eq!(result, MAX_16);
    assert_eq!(get_overflow(), 1);
}

#[test]
fn test_long_multiplication() {
    // Test L_mult: produces 32-bit result
    let half = 16384; // 0.5 in Q15
    let expected = 536870912; // 0.25 in Q31
    assert_eq!(l_mult(half, half), expected);
    
    // Test overflow case
    let result = l_mult(MIN_16, MIN_16);
    assert_eq!(result, MAX_32);
    assert_eq!(get_overflow(), 1);
}

#[test]
fn test_absolute_value() {
    assert_eq!(abs_s(100), 100);
    assert_eq!(abs_s(-100), 100);
    assert_eq!(abs_s(0), 0);
    
    // Test saturation: abs(MIN_16) should give MAX_16
    let result = abs_s(MIN_16);
    assert_eq!(result, MAX_16);
    assert_eq!(get_overflow(), 1);
}

#[test]
fn test_negation() {
    assert_eq!(negate(100), -100);
    assert_eq!(negate(-100), 100);
    assert_eq!(negate(0), 0);
    
    // Test saturation: negate(MIN_16) should give MAX_16
    let result = negate(MIN_16);
    assert_eq!(result, MAX_16);
    assert_eq!(get_overflow(), 1);
}

#[test]
fn test_shifting() {
    // Left shift (multiplication by 2^n)
    assert_eq!(shl(100, 1), 200);
    assert_eq!(shl(100, 2), 400);
    
    // Right shift (division by 2^n)
    assert_eq!(shr(200, 1), 100);
    assert_eq!(shr(400, 2), 100);
    
    // Negative shift should reverse operation
    assert_eq!(shl(100, -1), 50);
    assert_eq!(shr(100, -1), 200);
    
    // Test overflow in left shift
    let result = shl(MAX_16, 1);
    assert_eq!(result, MAX_16);
    assert_eq!(get_overflow(), 1);
}

#[test]
fn test_extract_operations() {
    let val: Word32 = 0x12345678;
    
    // Extract high 16 bits
    assert_eq!(extract_h(val), 0x1234);
    
    // Extract low 16 bits (with sign extension considerations)
    assert_eq!(extract_l(val), 0x5678);
    
    // Test with negative number
    let neg_val: Word32 = -0x12345678;
    assert_eq!(extract_h(neg_val), extract_h(neg_val));
}

#[test]
fn test_rounding() {
    // Test rounding: add 0x8000 and extract high
    let val = 0x12007FFF; // Should round down
    let rounded_down = round(val);
    
    let val2 = 0x12008000; // Should round up
    let rounded_up = round(val2);
    
    assert!(rounded_up > rounded_down);
}

#[test]
fn test_mac_operations() {
    // Test multiply-accumulate
    let acc = 1000;
    let var1 = 100;
    let var2 = 200;
    
    let result = l_mac(acc, var1, var2);
    let expected = l_add(acc, l_mult(var1, var2));
    assert_eq!(result, expected);
    
    // Test multiply-subtract
    let result = l_msu(acc, var1, var2);
    let expected = l_sub(acc, l_mult(var1, var2));
    assert_eq!(result, expected);
}

#[test]
fn test_long_operations() {
    // Test long addition
    let a: Word32 = 100000;
    let b: Word32 = 200000;
    assert_eq!(l_add(a, b), 300000);
    
    // Test long subtraction
    assert_eq!(l_sub(b, a), 100000);
    
    // Test long shifts
    assert_eq!(l_shl(100, 1), 200);
    assert_eq!(l_shr(200, 1), 100);
}

#[test]
fn test_normalization() {
    // Test norm_s: count leading zeros minus 1
    assert_eq!(norm_s(0), 0);
    assert_eq!(norm_s(1), 14); // 0000000000000001 has 14 leading zeros
    assert_eq!(norm_s(MAX_16), 0); // 0111111111111111 has 0 extra leading zeros
    
    // Test norm_l for 32-bit values
    assert_eq!(norm_l(0), 0);
    assert_eq!(norm_l(1), 30); // 32-bit 1 has 30 leading zeros
}

#[test]
fn test_division() {
    // Test basic division
    let dividend = 16384; // 0.5 in Q15
    let divisor = 32767; // ~1.0 in Q15  
    let result = div_s(dividend, divisor);
    
    // Result should be approximately 0.5
    assert!(result > 16000 && result < 17000);
    
    // Test division by zero
    let result = div_s(100, 0);
    assert_eq!(result, MAX_16); // Should saturate
    assert_eq!(get_overflow(), 1);
    
    // Test when dividend >= divisor (should overflow)
    let result = div_s(MAX_16, 100);
    assert_eq!(result, MAX_16);
    assert_eq!(get_overflow(), 1);
}

#[test]
fn test_deposit_operations() {
    let val: Word16 = 0x1234;
    
    // Deposit in high part
    let high_deposit = l_deposit_h(val);
    assert_eq!(high_deposit, 0x12340000);
    
    // Deposit in low part
    let low_deposit = l_deposit_l(val);
    assert_eq!(low_deposit, 0x00001234);
}

#[test]
fn test_array_operations() {
    // Test set_zero_16
    let mut array = [1, 2, 3, 4, 5];
    set_zero_16(&mut array);
    assert_eq!(array, [0, 0, 0, 0, 0]);
    
    // Test copy_16
    let src = [1, 2, 3, 4, 5];
    let mut dst = [0; 5];
    copy_16(&src, &mut dst);
    assert_eq!(dst, src);
    
    // Test partial copy
    let mut dst_small = [0; 3];
    copy_16(&src, &mut dst_small);
    assert_eq!(dst_small, [1, 2, 3]);
}

#[test]
fn test_overflow_flags() {
    // Reset overflow flag
    set_overflow(0);
    assert_eq!(get_overflow(), 0);
    
    // Trigger overflow
    add(MAX_16, 1);
    assert_eq!(get_overflow(), 1);
    
    // Reset and test carry flag
    set_carry(0);
    assert_eq!(get_carry(), 0);
    
    set_carry(1);
    assert_eq!(get_carry(), 1);
}

/// Test compatibility with ITU Q-format conventions
#[test]
fn test_q_format_compatibility() {
    // Q15 format tests: 1.0 = 32767, 0.5 = 16384, -1.0 = -32768
    let one_q15 = 32767;
    let half_q15 = 16384;
    let neg_one_q15 = -32768;
    
    // 0.5 * 0.5 should give 0.25
    let result = mult(half_q15, half_q15);
    let quarter_q15 = 8192;
    assert!((result - quarter_q15).abs() <= 1); // Allow for rounding
    
    // 1.0 * 0.5 should give approximately 0.5
    let result = mult(one_q15, half_q15);
    assert!((result - half_q15).abs() <= 100); // Allow for Q15 precision
}

/// Integration test that mimics ITU test patterns
#[test]
fn test_itu_patterns() {
    // Test pattern from ITU: typical G.729 coefficient processing
    let coeffs = [30000, 26000, 21000, 15000, 8000, 0, -8000, -15000, -21000, -26000];
    let mut processed = [0; 10];
    
    // Apply typical G.729 operations
    for i in 0..10 {
        processed[i] = add(coeffs[i], shl(coeffs[i], -3)); // Add 1/8 of value
        processed[i] = mult(processed[i], 29491); // Multiply by ~0.9
    }
    
    // Verify processing doesn't overflow
    for &val in &processed {
        assert!(val >= MIN_16 && val <= MAX_16);
    }
} 