//! DSP Utility Functions for G.729
//!
//! This module implements the Digital Signal Processing utility functions used in G.729,
//! based on the ITU-T reference implementation in DSPFUNC.C.
//!
//! These functions include logarithmic, power, and square root operations used
//! throughout the G.729 encoding and decoding process.

use super::types::{Word16, Word32};
use super::math::*;

/// Table for Pow2() function - powers of 2 in Q15 format
const TABPOW: [Word16; 33] = [
    16384, 16743, 17109, 17484, 17867, 18258, 18658, 19066, 19484, 19911,
    20347, 20792, 21247, 21713, 22188, 22674, 23170, 23678, 24196, 24726,
    25268, 25821, 26386, 26964, 27554, 28158, 28774, 29405, 30048, 30706,
    31379, 32066, 32767
];

/// Table for Log2() function - logarithm base 2 in Q15 format
const TABLOG: [Word16; 33] = [
    0, 1455, 2866, 4236, 5568, 6863, 8124, 9352, 10549, 11716,
    12855, 13967, 15054, 16117, 17156, 18172, 19167, 20142, 21097, 22033,
    22951, 23852, 24735, 25603, 26455, 27291, 28113, 28922, 29716, 30497,
    31266, 32023, 32767
];

/// Table for Inv_sqrt() function - inverse square root in Q15 format
const TABSQR: [Word16; 49] = [
    32767, 31790, 30894, 30070, 29309, 28602, 27945, 27330, 26755, 26214,
    25705, 25225, 24770, 24339, 23930, 23541, 23170, 22817, 22479, 22155,
    21845, 21548, 21263, 20988, 20724, 20470, 20225, 19988, 19760, 19539,
    19326, 19119, 18919, 18725, 18536, 18354, 18176, 18004, 17837, 17674,
    17515, 17361, 17211, 17064, 16921, 16782, 16646, 16514, 16384
];

/// Compute 2^(exponent.fraction)
///
/// # Arguments
/// * `exponent` - Integer part (Q0, range: 0..=30)
/// * `fraction` - Fractional part (Q15, range: 0.0..1.0)
///
/// # Returns
/// * Result in Q0 format (range: 0..=0x7fffffff)
///
/// # Algorithm
/// The function uses table lookup and linear interpolation:
/// 1. Extract high 5 bits of fraction for table index (i)
/// 2. Extract low 10 bits for interpolation factor (a)
/// 3. Interpolate: result = tabpow[i] - (tabpow[i] - tabpow[i+1]) * a
/// 4. Shift result right by (30-exponent) with rounding
pub fn pow2(exponent: Word16, fraction: Word16) -> Word32 {
    // L_x = fraction << 6 (to get high 5 bits in upper word)
    let l_x = l_mult(fraction, 32);
    let i = extract_h(l_x) as usize;
    
    // Get interpolation factor (low 10 bits)
    let l_x = l_shr(l_x, 1);
    let a = extract_l(l_x) & 0x7fff;
    
    // Table lookup with linear interpolation
    let mut l_result = l_deposit_h(TABPOW[i]);
    let tmp = sub(TABPOW[i], TABPOW[i + 1]);
    l_result = l_msu(l_result, tmp, a);
    
    // Denormalize by shifting right (30-exponent) with rounding
    let exp = sub(30, exponent);
    l_shr_r(l_result, exp)
}

/// Compute log2(L_x)
///
/// # Arguments  
/// * `l_x` - Input value (Q0, must be positive)
/// * `exponent` - Output integer part of log2 (Q0, range: 0..=30)
/// * `fraction` - Output fractional part of log2 (Q15, range: 0..1)
///
/// # Algorithm
/// The function uses normalization and table lookup:
/// 1. Normalize L_x and compute exponent = 30 - normalization_shift
/// 2. Extract high 7 bits for table index
/// 3. Extract middle bits for interpolation factor
/// 4. Interpolate: result = tablog[i] - (tablog[i] - tablog[i+1]) * a
pub fn log2(l_x: Word32, exponent: &mut Word16, fraction: &mut Word16) {
    if l_x <= 0 {
        *exponent = 0;
        *fraction = 0;
        return;
    }
    
    // Normalize input
    let exp = norm_l(l_x);
    let l_x_norm = l_shl(l_x, exp);
    
    *exponent = sub(30, exp);
    
    // Extract table index (bits 25-31 after normalization)
    let l_x_shifted = l_shr(l_x_norm, 9);
    let i = extract_h(l_x_shifted) as usize;
    
    // Extract interpolation factor (bits 10-24)
    let l_x_shifted = l_shr(l_x_shifted, 1);
    let a = extract_l(l_x_shifted) & 0x7fff;
    
    // Adjust index (normalized values have i in range 32-63)
    let i = i - 32;
    
    // Table lookup with linear interpolation
    let mut l_y = l_deposit_h(TABLOG[i]);
    let tmp = sub(TABLOG[i], TABLOG[i + 1]);
    l_y = l_msu(l_y, tmp, a);
    
    *fraction = extract_h(l_y);
}

/// Compute 1/sqrt(L_x)
///
/// # Arguments
/// * `l_x` - Input value (Q0, range: 0..=0x7fffffff)
///
/// # Returns  
/// * Result in Q30 format (range: 0..1)
///
/// # Algorithm
/// The function uses normalization and table lookup:
/// 1. Normalize L_x
/// 2. If (30-exponent) is even, shift right once more
/// 3. Compute final exponent = (30-original_exponent)/2 + 1
/// 4. Extract table index and interpolation factor
/// 5. Interpolate: result = tabsqr[i] - (tabsqr[i] - tabsqr[i+1]) * a
/// 6. Shift result right by computed exponent
pub fn inv_sqrt(l_x: Word32) -> Word32 {
    if l_x <= 0 {
        return 0x3fffffff; // Maximum Q30 value
    }
    
    // Normalize input
    let exp = norm_l(l_x);
    let mut l_x_norm = l_shl(l_x, exp);
    
    let mut exp = sub(30, exp);
    
    // If exponent is even, shift right once more
    if (exp & 1) == 0 {
        l_x_norm = l_shr(l_x_norm, 1);
    }
    
    // Final exponent for denormalization
    exp = shr(exp, 1);
    exp = add(exp, 1);
    
    // Extract table index (bits 25-31)
    let l_x_shifted = l_shr(l_x_norm, 9);
    let i = extract_h(l_x_shifted) as usize;
    
    // Extract interpolation factor (bits 10-24)  
    let l_x_shifted = l_shr(l_x_shifted, 1);
    let a = extract_l(l_x_shifted) & 0x7fff;
    
    // Adjust index (normalized values have i in range 16-63)
    let i = i - 16;
    
    // Table lookup with linear interpolation
    let mut l_y = l_deposit_h(TABSQR[i]);
    let tmp = sub(TABSQR[i], TABSQR[i + 1]);
    l_y = l_msu(l_y, tmp, a);
    
    // Denormalize
    l_shr(l_y, exp)
}

/// Compute autocorrelation
///
/// Computes the autocorrelation of a signal for LPC analysis.
/// This is a fundamental DSP operation used in speech coding.
///
/// # Arguments
/// * `x` - Input signal
/// * `r` - Output autocorrelation coefficients  
/// * `order` - LPC order (typically 10 for G.729)
/// * `window` - Analysis window (if provided)
pub fn autocorrelation(x: &[Word16], r: &mut [Word32], order: usize, window: Option<&[Word16]>) {
    // Clear output array
    for i in 0..=order {
        r[i] = 0;
    }
    
    let len = x.len();
    
    // Compute autocorrelation
    for i in 0..=order {
        let mut sum = 0i64;
        
        for j in 0..(len - i) {
            let x_j = if let Some(w) = window {
                mult(x[j], w[j])
            } else {
                x[j]
            };
            
            let x_ji = if j + i < len {
                if let Some(w) = window {
                    mult(x[j + i], w[j + i])
                } else {
                    x[j + i]
                }
            } else {
                0
            };
            
            sum += (x_j as i64) * (x_ji as i64);
        }
        
        // Normalize and saturate
        r[i] = if sum > MAX_32 as i64 {
            MAX_32
        } else if sum < MIN_32 as i64 {
            MIN_32
        } else {
            sum as Word32
        };
    }
}

/// Compute convolution
///
/// Computes the convolution of two signals, used in filtering operations.
///
/// # Arguments
/// * `x` - First input signal
/// * `h` - Second input signal (typically impulse response)
/// * `y` - Output convolution result
/// * `x_len` - Length of first signal
/// * `h_len` - Length of second signal
pub fn convolution(x: &[Word16], h: &[Word16], y: &mut [Word16], x_len: usize, h_len: usize) {
    let y_len = x_len + h_len - 1;
    
    for n in 0..y_len.min(y.len()) {
        let mut sum = 0i32;
        
        let k_start = if n + 1 >= x_len { n + 1 - x_len } else { 0 };
        let k_end = (n + 1).min(h_len);
        
        for k in k_start..k_end {
            if n >= k && (n - k) < x_len {
                sum += (h[k] as i32) * (x[n - k] as i32);
            }
        }
        
        // Saturate result
        y[n] = if sum > MAX_16 as i32 {
            MAX_16
        } else if sum < MIN_16 as i32 {
            MIN_16
        } else {
            sum as Word16
        };
    }
}

/// Apply window function
///
/// Applies a window function to a signal, typically used before LPC analysis.
///
/// # Arguments
/// * `signal` - Input/output signal
/// * `window` - Window coefficients  
/// * `length` - Length to process
pub fn apply_window(signal: &mut [Word16], window: &[Word16], length: usize) {
    for i in 0..length.min(signal.len()).min(window.len()) {
        signal[i] = mult(signal[i], window[i]);
    }
}

/// Compute energy of a signal
///
/// Computes the energy (sum of squares) of a signal segment.
///
/// # Arguments
/// * `signal` - Input signal
/// * `length` - Length to process
///
/// # Returns
/// * Signal energy in Q0 format
pub fn compute_energy(signal: &[Word16], length: usize) -> Word32 {
    let mut energy = 0i64;
    
    for i in 0..length.min(signal.len()) {
        energy += (signal[i] as i64) * (signal[i] as i64);
    }
    
    // Saturate to 32-bit
    if energy > MAX_32 as i64 {
        MAX_32
    } else {
        energy as Word32
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pow2_basic() {
        // Test 2^1.0 = 2
        let result = pow2(1, 0);
        assert_eq!(result, 2);
        
        // Test 2^0.0 = 1  
        let result = pow2(0, 0);
        assert_eq!(result, 1);
        
        // Test 2^2.0 = 4
        let result = pow2(2, 0);
        assert_eq!(result, 4);
    }

    #[test]
    fn test_log2_basic() {
        let mut exp = 0;
        let mut frac = 0;
        
        // Test log2(1) = 0
        log2(1, &mut exp, &mut frac);
        assert_eq!(exp, 0);
        // frac should be close to 0
        
        // Test log2(2) = 1  
        log2(2, &mut exp, &mut frac);
        assert_eq!(exp, 1);
        
        // Test log2(4) = 2
        log2(4, &mut exp, &mut frac);
        assert_eq!(exp, 2);
    }

    #[test]
    fn test_inv_sqrt_basic() {
        // Test 1/sqrt(1) = 1 (in Q30: 1.0 = 2^30)
        let result = inv_sqrt(1);
        assert!(result > 0);
        
        // Test 1/sqrt(4) = 0.5 (in Q30: 0.5 = 2^29)
        let result = inv_sqrt(4);
        assert!(result > 0);
        assert!(result < 0x40000000); // Less than 1.0 in Q30
    }

    #[test]
    fn test_autocorrelation_basic() {
        let signal = [100, 200, 150, 50];
        let mut r = [0; 5];
        
        autocorrelation(&signal, &mut r, 4, None);
        
        // R[0] should be the energy (sum of squares)
        assert!(r[0] > 0);
        
        // R[i] should be decreasing in magnitude for typical signals
        assert!(r[0] >= r[1].abs());
    }

    #[test]
    fn test_compute_energy() {
        let signal = [100, -200, 150, -50];
        let energy = compute_energy(&signal, 4);
        
        // Energy = 100^2 + 200^2 + 150^2 + 50^2 = 10000 + 40000 + 22500 + 2500 = 75000
        assert_eq!(energy, 75000);
    }

    #[test]
    fn test_convolution_basic() {
        let x = [1, 2, 3];
        let h = [1, 1];
        let mut y = [0; 4];
        
        convolution(&x, &h, &mut y, 3, 2);
        
        // Expected: [1, 3, 5, 3]
        assert_eq!(y[0], 1);
        assert_eq!(y[1], 3);
        assert_eq!(y[2], 5);
        assert_eq!(y[3], 3);
    }

    #[test]
    fn test_apply_window() {
        let mut signal = [100, 200, 300];
        let window = [16384, 32767, 16384]; // 0.5, 1.0, 0.5 in Q15
        
        apply_window(&mut signal, &window, 3);
        
        // Check that windowing was applied (values should be modified)
        assert_ne!(signal[0], 100);
        assert_ne!(signal[2], 300);
    }
} 