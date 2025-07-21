//! DSP operations for G.729A codec

use crate::codecs::g729a::types::{Q15, Q31};
use crate::codecs::g729a::math::fixed_point::FixedPointOps;

/// ITU-T countLeadingZeros function
/// MSB is excluded as considered sign bit
/// Returns number of leading zeros (MSB excluded)
/// Example: 0x00800000 returns 7
fn count_leading_zeros_itu_t(x: u32) -> i32 {
    if x == 0 {
        return 31;
    }
    
    let mut leading_zeros = 0i32;
    let mut value = x;
    
    // Count zeros until reaching bit 30 (0x40000000)
    while value < 0x40000000 {
        leading_zeros += 1;
        value <<= 1;
    }
    
    leading_zeros
}

/// ITU-T ADD32 - 32-bit saturated addition
pub fn add32(a: i32, b: i32) -> i32 {
    let result = a.saturating_add(b);
    result
}

/// ITU-T SUB32 - 32-bit saturated subtraction  
pub fn sub32(a: i32, b: i32) -> i32 {
    let result = a.saturating_sub(b);
    result
}

/// ITU-T SHR32 - 32-bit right shift
pub fn shr32(a: i32, shift: i32) -> i32 {
    if shift >= 32 {
        if a >= 0 { 0 } else { -1 }
    } else if shift > 0 {
        a >> shift
    } else {
        a
    }
}

/// ITU-T SHL32 - 32-bit left shift with saturation
pub fn shl32(a: i32, shift: i32) -> i32 {
    if shift >= 32 {
        0
    } else if shift > 0 {
        let result = (a as i64) << shift;
        if result > i32::MAX as i64 {
            i32::MAX
        } else if result < i32::MIN as i64 {
            i32::MIN
        } else {
            result as i32
        }
    } else {
        a
    }
}

/// ITU-T MAC16_16 - 16-bit multiply-accumulate to 32-bit
pub fn mac16_16(acc: i32, a: i16, b: i16) -> i32 {
    let product = (a as i32) * (b as i32);
    acc.saturating_add(product)
}

/// ITU-T MULT32_32_Q23 - 32-bit multiply in Q23 format
pub fn mult32_32_q23(a: i32, b: i32) -> i32 {
    let product = ((a as i64) * (b as i64)) >> 23;
    if product > i32::MAX as i64 {
        i32::MAX
    } else if product < i32::MIN as i64 {
        i32::MIN
    } else {
        product as i32
    }
}

/// ITU-T g729InvSqrt_Q0Q31 - Inverse square root for pitch normalization
/// Input: x in Q0, Output: 1/sqrt(x) in Q31
pub fn g729_inv_sqrt_q0q31(x: i32) -> i32 {
    if x <= 0 {
        return i32::MAX; // Return maximum for invalid input
    }
    
    // ITU-T uses table-based approximation + Newton-Raphson
    // Simplified implementation using integer approximation
    let x_u32 = x as u32;
    
    // Find approximate inverse square root using bit manipulation
    let leading_zeros = count_leading_zeros_itu_t(x_u32);
    let normalized_x = x_u32 << leading_zeros;
    
    // Use lookup table approximation (simplified)
    let index = ((normalized_x >> 25) & 0x3F) as usize; // 6-bit index
    let inv_sqrt_approx = INV_SQRT_TABLE[index];
    
    // Adjust for normalization
    let shift_adjust = (leading_zeros + 1) >> 1; // Half the normalization shift
    let result = if shift_adjust < 15 {
        inv_sqrt_approx >> (15 - shift_adjust)
    } else {
        inv_sqrt_approx << (shift_adjust - 15)
    };
    
    result.min(i32::MAX)
}

/// Lookup table for inverse square root (6-bit precision)
const INV_SQRT_TABLE: [i32; 64] = [
    32767, 31790, 30894, 30070, 29309, 28602, 27945, 27330,
    26755, 26214, 25705, 25225, 24770, 24339, 23930, 23541,
    23170, 22817, 22479, 22155, 21845, 21548, 21263, 20988,
    20724, 20470, 20225, 19988, 19760, 19539, 19326, 19119,
    18919, 18725, 18536, 18354, 18176, 18004, 17837, 17674,
    17515, 17361, 17211, 17064, 16921, 16782, 16646, 16514,
    16384, 16257, 16134, 16013, 15895, 15779, 15666, 15555,
    15446, 15340, 15235, 15132, 15031, 14932, 14835, 14740
];

/// Compute autocorrelation with exact ITU-T G.729A algorithm
/// Implements dynamic scaling and 64-bit accumulation per spec 3.2.1
/// Returns correlation values normalized to 32-bit range
pub fn autocorrelation(windowed_signal: &[Q15], order: usize) -> Vec<Q31> {
    let len = windowed_signal.len();
    let mut corr = Vec::with_capacity(order + 1);
    
    // ITU-T: Compute R[0] first using 64-bit accumulation
    let mut acc64 = 0i64;
    for i in 0..len {
        let sample = windowed_signal[i].0 as i32;
        acc64 += (sample as i64) * (sample as i64);
    }
    
    // ITU-T: R[0] must have minimum value of 1.0 to avoid arithmetic problems
    if acc64 == 0 {
        acc64 = 1;
    }
    
    // ITU-T: Normalize acc64 to fit in 32-bit and track scaling (exact algorithm)
    let mut right_shift_to_normalize = 0i32;
    let r0_normalized = if acc64 > i32::MAX as i64 {
        // ITU-T: acc64 > MAXINT32, shift right until it fits
        let mut temp_acc = acc64;
        while temp_acc > i32::MAX as i64 {
            temp_acc >>= 1;
            right_shift_to_normalize += 1;
        }
        temp_acc as i32
    } else {
        // ITU-T: rightShiftToNormalise = -countLeadingZeros((word32_t)acc64)
        let acc32 = acc64 as u32;
        let leading_zeros = count_leading_zeros_itu_t(acc32);
        right_shift_to_normalize = -leading_zeros;
        
        // ITU-T: SSHL((word32_t)acc64, -rightShiftToNormalise)
        let shift_amount = -right_shift_to_normalize;
        if shift_amount > 0 && shift_amount < 31 { // Keep one bit for sign
            let shifted = (acc64 << shift_amount).min(i32::MAX as i64);
            shifted as i32
        } else if shift_amount < 0 {
            (acc64 >> (-shift_amount)) as i32
        } else {
            (acc64 as i64).min(i32::MAX as i64) as i32
        }
    };
    
    corr.push(Q31(r0_normalized));
    
    #[cfg(debug_assertions)]
    {
        eprintln!("ITU-T Autocorrelation debug:");
        eprintln!("  Raw R[0]: {}, Normalized: {}, Scale: {}", acc64, r0_normalized, right_shift_to_normalize);
    }
    
    // ITU-T: Compute R[1] to R[order] with same scaling
    for lag in 1..=order {
        let mut acc64 = 0i64;
        let max_i = len - lag;
        
        for i in 0..max_i {
            let sample1 = windowed_signal[i].0 as i32;
            let sample2 = windowed_signal[i + lag].0 as i32;
            acc64 += (sample1 as i64) * (sample2 as i64);
        }
        
        // Apply same scaling as R[0]
        let r_lag = if right_shift_to_normalize > 0 {
            (acc64 >> right_shift_to_normalize) as i32
        } else if right_shift_to_normalize < 0 {
            let shift_amount = -right_shift_to_normalize;
            if shift_amount < 31 {
                let shifted = (acc64 << shift_amount).min(i32::MAX as i64);
                shifted as i32
            } else {
                (acc64 as i64).min(i32::MAX as i64) as i32
            }
        } else {
            (acc64 as i64).min(i32::MAX as i64) as i32
        };
        
        corr.push(Q31(r_lag));
    }
    
    #[cfg(debug_assertions)]
    {
        eprintln!("  R[1]: {}, R[2]: {}", corr[1].0, corr[2].0);
    }
    
    corr
}

/// Apply lag window to autocorrelation coefficients per ITU-T spec 3.2.1 eq7
pub fn apply_lag_window(corr: &mut [Q31], lag_window: &[Q15]) {
    // ITU-T: Apply lag window starting from R[1] (R[0] remains unchanged)
    for i in 1..corr.len().min(lag_window.len()) {
        // corr[i] = MULT16_32_P15(wlag[i], corr[i])
        let windowed = mult16_32_p15(lag_window[i].0, corr[i].0);
        corr[i] = Q31(windowed);
    }
}

/// ITU-T MULT16_32_P15 operation: (a * b) >> 15 with rounding
fn mult16_32_p15(a: i16, b: i32) -> i32 {
    let product = (a as i64) * (b as i64);
    ((product + 0x4000) >> 15) as i32
}

/// ITU-T MULT16_16_P15 operation for windowing
pub fn mult16_16_p15(a: i16, b: i16) -> i16 {
    let product = (a as i32) * (b as i32);
    ((product + 0x4000) >> 15) as i16
}

/// Optimized convolution for filter operations
pub fn convolution(x: &[Q15], h: &[Q15]) -> Vec<Q15> {
    let x_len = x.len();
    let h_len = h.len();
    let y_len = x_len + h_len - 1;
    let mut y = vec![Q15::ZERO; y_len];
    
    // y[n] = sum(x[k] * h[n-k])
    for n in 0..y_len {
        let mut sum = Q31::ZERO;
        
        let k_start = n.saturating_sub(h_len - 1);
        let k_end = (n + 1).min(x_len);
        
        for k in k_start..k_end {
            if n >= k && (n - k) < h_len {
                let prod = x[k].to_q31().saturating_mul(h[n - k].to_q31());
                sum = sum.saturating_add(prod);
            }
        }
        
        y[n] = sum.to_q15();
    }
    
    y
}

/// Inner product with accumulation in Q31
pub fn dot_product(a: &[Q15], b: &[Q15]) -> Q31 {
    assert_eq!(a.len(), b.len(), "Vectors must have same length");
    
    let mut sum = Q31::ZERO;
    
    for i in 0..a.len() {
        let prod = a[i].to_q31().saturating_mul(b[i].to_q31());
        sum = sum.saturating_add(prod);
    }
    
    sum
}

/// Compute cross-correlation between two signals
pub fn cross_correlation(x: &[Q15], y: &[Q15], lag: usize) -> Q31 {
    let len = x.len().min(y.len() - lag);
    let mut sum = Q31::ZERO;
    
    for i in 0..len {
        let prod = x[i].to_q31().saturating_mul(y[i + lag].to_q31());
        sum = sum.saturating_add(prod);
    }
    
    sum
}

/// Compute energy of a signal
pub fn energy(signal: &[Q15]) -> Q31 {
    let mut sum = Q31::ZERO;
    
    for &sample in signal {
        // Compute sample^2 with proper Q-format scaling
        // Q15 * Q15 = Q30, but we want Q31 result  
        let sample_i32 = sample.0 as i32;
        let square_q30 = sample_i32 * sample_i32; // Q15*Q15 = Q30
        let square_q31 = Q31(square_q30 >> 1); // Convert Q30 to Q31 (divide by 2)
        sum = sum.saturating_add(square_q31);
    }
    
    sum
}

/// Normalize a vector by its energy
pub fn normalize_vector(signal: &[Q15]) -> Vec<Q15> {
    let e = energy(signal);
    
    if e.0 == 0 {
        return vec![Q15::ZERO; signal.len()];
    }
    
    // Compute 1/sqrt(energy)
    let inv_norm = crate::codecs::g729a::math::fixed_point::inverse_sqrt(e);
    
    signal.iter()
        .map(|&x| x.saturating_mul(inv_norm))
        .collect()
}

/// Apply first-order IIR filter
/// y[n] = b0*x[n] + b1*x[n-1] - a1*y[n-1]
pub fn iir_filter_1st_order(
    x: &[Q15], 
    b0: Q15, 
    b1: Q15, 
    a1: Q15,
    state: &mut (Q15, Q15) // (x[n-1], y[n-1])
) -> Vec<Q15> {
    let mut y = Vec::with_capacity(x.len());
    
    for &xn in x {
        // Compute filter output
        let b0_xn = b0.saturating_mul(xn);
        let b1_xn1 = b1.saturating_mul(state.0);
        let a1_yn1 = a1.saturating_mul(state.1);
        
        let yn = b0_xn.saturating_add(b1_xn1).saturating_add(Q15(-a1_yn1.0));
        
        y.push(yn);
        
        // Update state
        state.0 = xn;
        state.1 = yn;
    }
    
    y
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_autocorrelation() {
        // Test with a simple signal
        let signal = vec![
            Q15::from_f32(0.5),
            Q15::from_f32(0.3),
            Q15::from_f32(-0.2),
            Q15::from_f32(0.1),
        ];
        
        let corr = autocorrelation(&signal, 2);
        
        // R(0) should be the signal energy
        let energy = dot_product(&signal, &signal);
        assert!((corr[0].0 - (energy.0 >> 1)).abs() < 1000);
        
        // R(k) should be symmetric and decreasing
        assert!(corr[0].0 > corr[1].0.abs());
        assert!(corr[1].0.abs() > corr[2].0.abs());
    }

    #[test]
    fn test_convolution() {
        // Test impulse response
        let x = vec![Q15::ONE, Q15::ZERO, Q15::ZERO];
        let h = vec![Q15::from_f32(0.5), Q15::from_f32(0.25)];
        
        let y = convolution(&x, &h);
        
        // Output should be h when convolving with impulse
        assert!((y[0].to_f32() - 0.5).abs() < 0.01);
        assert!((y[1].to_f32() - 0.25).abs() < 0.01);
        assert_eq!(y[2], Q15::ZERO);
    }

    #[test]
    fn test_dot_product() {
        let a = vec![Q15::from_f32(0.5), Q15::from_f32(0.5)];
        let b = vec![Q15::from_f32(0.5), Q15::from_f32(0.5)];
        
        let result = dot_product(&a, &b);
        
        // 0.5*0.5 + 0.5*0.5 = 0.5
        assert!((result.to_f32() - 0.5).abs() < 0.01);
    }

    #[test]
    fn test_energy() {
        let signal = vec![Q15::from_f32(0.5), Q15::from_f32(0.5)];
        let e = energy(&signal);
        
        // 0.5^2 + 0.5^2 = 0.5
        assert!((e.to_f32() - 0.5).abs() < 0.01);
    }

    #[test]
    fn test_normalize_vector() {
        let signal = vec![Q15::from_f32(0.6), Q15::from_f32(0.8)];
        let normalized = normalize_vector(&signal);
        
        // Check that normalized energy is close to 1
        let norm_energy = energy(&normalized);
        assert!((norm_energy.to_f32() - 0.999).abs() < 0.1);
        
        // Check direction is preserved
        let ratio = signal[0].0 as f32 / signal[1].0 as f32;
        let norm_ratio = normalized[0].0 as f32 / normalized[1].0 as f32;
        assert!((ratio - norm_ratio).abs() < 0.1);
    }

    #[test]
    fn test_iir_filter() {
        // Simple high-pass filter: H(z) = (1 - z^-1) / (1 - 0.9*z^-1)
        let b0 = Q15::ONE;
        let b1 = Q15::from_f32(-0.999);
        let a1 = Q15::from_f32(-0.9);
        
        let x = vec![Q15::ONE, Q15::ZERO, Q15::ZERO, Q15::ZERO];
        let mut state = (Q15::ZERO, Q15::ZERO);
        
        let y = iir_filter_1st_order(&x, b0, b1, a1, &mut state);
        
        // First output should be close to 1
        assert!((y[0].to_f32() - 0.999).abs() < 0.1);
        
        // Output should decay
        assert!(y[1].0.abs() < y[0].0.abs());
        assert!(y[2].0.abs() < y[1].0.abs());
    }
} 