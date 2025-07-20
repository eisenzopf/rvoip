//! DSP operations for G.729A codec

use crate::codecs::g729a::types::{Q15, Q31};
use crate::codecs::g729a::math::fixed_point::FixedPointOps;

/// Compute autocorrelation with lag windowing
/// Returns correlation values in Q31 format
pub fn autocorrelation(signal: &[Q15], order: usize) -> Vec<Q31> {
    let len = signal.len();
    let mut corr = Vec::with_capacity(order + 1);
    
    // R(k) = sum(s[n] * s[n-k]) for k = 0..order
    for k in 0..=order {
        let mut sum = Q31::ZERO;
        
        for n in k..len {
            // Compute product properly: Q15 * Q15 = Q30, then convert to Q31
            let sample1 = signal[n].0 as i32;
            let sample2 = signal[n - k].0 as i32;
            let prod_q30 = sample1 * sample2; // Q15 * Q15 = Q30
            let prod_q31 = Q31(prod_q30 >> 1); // Convert Q30 to Q31 (divide by 2)
            sum = sum.saturating_add(prod_q31);
        }
        
        corr.push(sum);
    }
    
    corr
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