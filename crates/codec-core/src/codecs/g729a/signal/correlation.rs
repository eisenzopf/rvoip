//! Correlation computation utilities

use crate::codecs::g729a::types::{Q15, Q31};
use crate::codecs::g729a::math::fixed_point::FixedPointOps;

/// Compute normalized cross-correlation for pitch detection
pub fn normalized_cross_correlation(x: &[Q15], y: &[Q15], lag: usize) -> Q15 {
    let len = x.len().min(y.len().saturating_sub(lag));
    
    if len == 0 {
        return Q15::ZERO;
    }
    
    // Compute cross-correlation
    let mut corr = Q31::ZERO;
    for i in 0..len {
        let prod = x[i].to_q31().saturating_mul(y[i + lag].to_q31());
        corr = corr.saturating_add(prod);
    }
    
    // Compute energies
    let mut energy_x = Q31::ZERO;
    let mut energy_y = Q31::ZERO;
    
    for i in 0..len {
        let x_sq = x[i].to_q31().saturating_mul(x[i].to_q31());
        energy_x = energy_x.saturating_add(x_sq);
        
        let y_sq = y[i + lag].to_q31().saturating_mul(y[i + lag].to_q31());
        energy_y = energy_y.saturating_add(y_sq);
    }
    
    // Avoid division by zero
    if energy_x.0 == 0 || energy_y.0 == 0 {
        return Q15::ZERO;
    }
    
    // Compute sqrt(Ex * Ey)
    let energy_prod = Q31((energy_x.0 as i64 * energy_y.0 as i64 >> 31) as i32);
    let inv_sqrt = crate::codecs::g729a::math::fixed_point::inverse_sqrt(energy_prod);
    
    // Normalize correlation
    let normalized = corr.to_q15().saturating_mul(inv_sqrt);
    normalized
}

/// Compute decimated correlation for efficient pitch search
pub fn decimated_correlation(signal: &[Q15], lag: usize, decimation: usize) -> Q31 {
    let len = signal.len().saturating_sub(lag);
    let mut sum = Q31::ZERO;
    
    // Only use samples at decimation intervals
    let mut i = 0;
    while i < len {
        let prod = signal[i].to_q31().saturating_mul(signal[i + lag].to_q31());
        sum = sum.saturating_add(prod);
        i += decimation;
    }
    
    sum
}

/// Compute backward correlation for target signal
pub fn backward_correlation(target: &[Q15], h: &[Q15]) -> Vec<Q15> {
    let len = target.len();
    let h_len = h.len();
    let mut d = vec![Q15::ZERO; len];
    
    // d[n] = sum(target[k] * h[k-n]) for k >= n
    for n in 0..len {
        let mut sum = Q31::ZERO;
        
        for k in n..len.min(n + h_len) {
            if k - n < h_len {
                let prod = target[k].to_q31().saturating_mul(h[k - n].to_q31());
                sum = sum.saturating_add(prod);
            }
        }
        
        d[n] = sum.to_q15();
    }
    
    d
}

/// Compute phi matrix for algebraic codebook search
pub struct PhiMatrix {
    values: Vec<Vec<Q15>>,
    size: usize,
}

impl PhiMatrix {
    /// Create phi matrix from impulse response
    pub fn from_impulse_response(h: &[Q15], size: usize) -> Self {
        let mut values = vec![vec![Q15::ZERO; size]; size];
        
        // phi[i][j] = sum(h[k-i] * h[k-j]) for k >= max(i,j)
        for i in 0..size {
            for j in i..size {
                let mut sum = Q31::ZERO;
                
                for k in j..size.min(j + h.len()) {
                    if k - i < h.len() && k - j < h.len() {
                        let prod = h[k - i].to_q31().saturating_mul(h[k - j].to_q31());
                        sum = sum.saturating_add(prod);
                    }
                }
                
                let value = sum.to_q15();
                values[i][j] = value;
                values[j][i] = value; // Symmetric matrix
            }
        }
        
        Self { values, size }
    }
    
    /// Get matrix element
    pub fn get(&self, i: usize, j: usize) -> Q15 {
        if i < self.size && j < self.size {
            self.values[i][j]
        } else {
            Q15::ZERO
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_normalized_cross_correlation() {
        // Test with identical signals (correlation = 1)
        let signal = vec![
            Q15::from_f32(0.5),
            Q15::from_f32(0.3),
            Q15::from_f32(-0.2),
            Q15::from_f32(0.1),
        ];
        
        let corr = normalized_cross_correlation(&signal, &signal, 0);
        assert!(corr.to_f32() > 0.9); // Should be close to 1
        
        // Test with zero lag on different signals
        let signal2 = vec![
            Q15::from_f32(-0.5),
            Q15::from_f32(-0.3),
            Q15::from_f32(0.2),
            Q15::from_f32(-0.1),
        ];
        
        let corr2 = normalized_cross_correlation(&signal, &signal2, 0);
        assert!(corr2.to_f32() < -0.9); // Should be close to -1 (negatively correlated)
    }

    #[test]
    fn test_decimated_correlation() {
        let signal = vec![
            Q15::from_f32(0.5),
            Q15::from_f32(0.4),
            Q15::from_f32(0.3),
            Q15::from_f32(0.2),
            Q15::from_f32(0.1),
            Q15::from_f32(0.0),
        ];
        
        // Test with decimation factor 2
        let corr_full = decimated_correlation(&signal, 1, 1);
        let corr_decimated = decimated_correlation(&signal, 1, 2);
        
        // Decimated should be less than full correlation
        assert!(corr_decimated.0.abs() < corr_full.0.abs());
        
        // But should still be positive for this signal
        assert!(corr_decimated.0 > 0);
    }

    #[test]
    fn test_backward_correlation() {
        let target = vec![
            Q15::from_f32(0.5),
            Q15::from_f32(0.3),
            Q15::from_f32(0.1),
        ];
        
        let h = vec![
            Q15::from_f32(0.8),
            Q15::from_f32(0.4),
        ];
        
        let d = backward_correlation(&target, &h);
        
        // Check size
        assert_eq!(d.len(), target.len());
        
        // First element should be largest (full overlap)
        assert!(d[0].0.abs() > d[2].0.abs());
    }

    #[test]
    fn test_phi_matrix() {
        let h = vec![
            Q15::from_f32(0.8),
            Q15::from_f32(0.4),
            Q15::from_f32(0.2),
        ];
        
        let phi = PhiMatrix::from_impulse_response(&h, 5);
        
        // Check symmetry
        assert_eq!(phi.get(0, 1), phi.get(1, 0));
        assert_eq!(phi.get(1, 2), phi.get(2, 1));
        
        // Diagonal elements should be positive
        assert!(phi.get(0, 0).0 > 0);
        assert!(phi.get(1, 1).0 > 0);
        
        // Elements should decay with distance from diagonal
        assert!(phi.get(0, 0).0 > phi.get(0, 2).0.abs());
    }

    #[test]
    fn test_correlation_edge_cases() {
        let signal = vec![Q15::from_f32(0.5); 4];
        
        // Test with lag greater than signal length
        let corr = normalized_cross_correlation(&signal, &signal, 10);
        assert_eq!(corr, Q15::ZERO);
        
        // Test with empty signal
        let empty: Vec<Q15> = vec![];
        let corr_empty = normalized_cross_correlation(&empty, &signal, 0);
        assert_eq!(corr_empty, Q15::ZERO);
        
        // Test with zero energy
        let zeros = vec![Q15::ZERO; 4];
        let corr_zero = normalized_cross_correlation(&zeros, &zeros, 0);
        assert_eq!(corr_zero, Q15::ZERO);
    }
}

/// Compute autocorrelation matrix (phi matrix) for algebraic codebook search
pub fn compute_phi_matrix(h: &[Q15]) -> [[Q31; crate::codecs::g729a::constants::SUBFRAME_SIZE]; crate::codecs::g729a::constants::SUBFRAME_SIZE] {
    use crate::codecs::g729a::constants::SUBFRAME_SIZE;
    let mut phi = [[Q31::ZERO; SUBFRAME_SIZE]; SUBFRAME_SIZE];
    
    // Compute phi[i][j] = sum(h[n-i] * h[n-j]) for j >= i
    for i in 0..SUBFRAME_SIZE {
        for j in i..SUBFRAME_SIZE {
            let mut sum = Q31::ZERO;
            
            for n in j..h.len().min(SUBFRAME_SIZE) {
                let h_ni = if n >= i { h[n - i] } else { Q15::ZERO };
                let h_nj = if n >= j { h[n - j] } else { Q15::ZERO };
                let prod = h_ni.to_q31().saturating_mul(h_nj.to_q31());
                sum = sum.saturating_add(prod);
            }
            
            phi[i][j] = sum;
            phi[j][i] = sum; // Symmetric matrix
        }
    }
    
    phi
} 