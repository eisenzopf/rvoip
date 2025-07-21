//! Perceptual weighting filter for G.729A

use crate::codecs::g729a::constants::*;
use crate::codecs::g729a::types::{Q15, Q31, LPCoefficients};
use crate::codecs::g729a::math::{FixedPointOps, convolution};

/// Perceptual weighting filter W(z) = A(z/γ1) / A(z/γ2)
/// For G.729A, simplified to W(z) = 1 / A(z/γ) with γ = 0.75
pub struct PerceptualWeightingFilter {
    /// Gamma factor for bandwidth expansion (0.75 for G.729A)
    gamma: Q15,
}

/// Weighted filter coefficients
pub struct WeightedFilter {
    pub coefficients: [Q15; LP_ORDER + 1],
}

impl PerceptualWeightingFilter {
    /// Create a new perceptual weighting filter
    pub fn new() -> Self {
        Self {
            gamma: Q15(Q15_GAMMA), // 0.75 in Q15
        }
    }
    
    /// Create filter coefficients from LP coefficients
    /// For G.729A: W(z) = 1 / A(z/γ)
    pub fn create_filter(&self, lp_coeffs: &LPCoefficients) -> WeightedFilter {
        let mut coefficients = [Q15::ZERO; LP_ORDER + 1];
        
        // A(z/γ) coefficients
        coefficients[0] = Q15::ONE;
        let mut gamma_power = self.gamma;
        
        for i in 0..LP_ORDER {
            coefficients[i + 1] = lp_coeffs.values[i].saturating_mul(gamma_power);
            gamma_power = gamma_power.saturating_mul(self.gamma);
        }
        
        #[cfg(debug_assertions)]
        {
            eprintln!("Weighting filter debug:");
            eprintln!("  Input LP coeffs [0..5]: {:?}", lp_coeffs.values[..5].iter().map(|x| x.0).collect::<Vec<_>>());
            eprintln!("  Gamma: {}, powers: {:?}", self.gamma.0, 
                (0..5).map(|i| {
                    let mut gp = self.gamma;
                    for _ in 0..i { gp = gp.saturating_mul(self.gamma); }
                    gp.0
                }).collect::<Vec<_>>());
            eprintln!("  Weighted coeffs [0..5]: {:?}", coefficients[..6].iter().map(|x| x.0).collect::<Vec<_>>());
        }
        
        WeightedFilter { coefficients }
    }
    
    /// Compute impulse response of W(z) = 1/A(z/γ)
    pub fn compute_impulse_response(&self, filter: &WeightedFilter) -> [Q15; SUBFRAME_SIZE] {
        let mut h = [Q15::ZERO; SUBFRAME_SIZE];
        
        // Initialize with unit impulse
        h[0] = Q15::ONE;
        
        // Compute impulse response by filtering unit impulse
        // h[n] = δ[n] - Σ(a[k] * h[n-k]) for k=1..LP_ORDER
        for n in 1..SUBFRAME_SIZE {
            let mut sum = Q31::ZERO;
            
            let max_k = n.min(LP_ORDER);
            for k in 1..=max_k {
                let a_k = filter.coefficients[k];
                let h_nk = h[n - k];
                let prod = a_k.to_q31().saturating_mul(h_nk.to_q31());
                sum = sum.saturating_add(prod);
            }
            
            h[n] = Q15::ZERO.saturating_add(Q15(-sum.to_q15().0));
        }
        
        h
    }
    
    /// Apply weighting filter to a signal
    pub fn filter_signal(&self, signal: &[Q15], filter: &WeightedFilter) -> Vec<Q15> {
        let mut weighted = Vec::with_capacity(signal.len());
        let mut state = [Q15::ZERO; LP_ORDER];
        
        #[cfg(debug_assertions)]
        {
            let input_energy: i32 = signal.iter().take(80).map(|&x| (x.0 as i32).pow(2) >> 15).sum();
            eprintln!("  Filter input energy (80 samples): {}", input_energy);
            eprintln!("  First 5 input samples: {:?}", signal.iter().take(5).map(|x| x.0).collect::<Vec<_>>());
        }
        
        for (idx, &sample) in signal.iter().enumerate() {
            // Compute filter output
            let mut sum = sample.to_q31();
            
            for k in 0..LP_ORDER {
                let a_k = filter.coefficients[k + 1];
                let state_k = state[k];
                let prod = a_k.to_q31().saturating_mul(state_k.to_q31());
                sum = sum.saturating_add(Q31(-prod.0));
            }
            
            let output = sum.to_q15();
            weighted.push(output);
            
            #[cfg(debug_assertions)]
            if idx < 5 {
                eprintln!("  Sample {}: input={}, output={}, sum={}", idx, sample.0, output.0, sum.0);
            }
            
            // Update state
            for i in (1..LP_ORDER).rev() {
                state[i] = state[i - 1];
            }
            state[0] = output;
        }
        
        #[cfg(debug_assertions)]
        {
            let output_energy: i32 = weighted.iter().take(80).map(|&x| (x.0 as i32).pow(2) >> 15).sum();
            eprintln!("  Filter output energy (80 samples): {}", output_energy);
        }
        
        weighted
    }
    
    /// Apply bandwidth expansion to LP coefficients
    pub fn apply_bandwidth_expansion(&self, lp_coeffs: &[Q15], gamma: Q15) -> [Q15; LP_ORDER] {
        let mut expanded = [Q15::ZERO; LP_ORDER];
        let mut gamma_power = gamma;
        
        for i in 0..LP_ORDER {
            expanded[i] = lp_coeffs[i].saturating_mul(gamma_power);
            gamma_power = gamma_power.saturating_mul(gamma);
        }
        
        expanded
    }
}

impl Default for PerceptualWeightingFilter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_weighting_filter_creation() {
        let filter = PerceptualWeightingFilter::new();
        assert_eq!(filter.gamma, Q15(Q15_GAMMA));
    }

    #[test]
    fn test_create_filter_coefficients() {
        let filter = PerceptualWeightingFilter::new();
        
        let lp_coeffs = LPCoefficients {
            values: [Q15::from_f32(-0.8), Q15::from_f32(0.2), Q15::ZERO, Q15::ZERO, 
                     Q15::ZERO, Q15::ZERO, Q15::ZERO, Q15::ZERO, Q15::ZERO, Q15::ZERO],
            reflection_coeffs: [Q15::ZERO; LP_ORDER],
        };
        
        let weighted = filter.create_filter(&lp_coeffs);
        
        // First coefficient should always be 1
        assert_eq!(weighted.coefficients[0], Q15::ONE);
        
        // Second coefficient should be -0.8 * 0.75 = -0.6
        assert!((weighted.coefficients[1].to_f32() + 0.6).abs() < 0.01);
        
        // Third coefficient should be 0.2 * 0.75^2 = 0.1125
        assert!((weighted.coefficients[2].to_f32() - 0.1125).abs() < 0.02);
    }

    #[test]
    fn test_impulse_response() {
        let filter = PerceptualWeightingFilter::new();
        
        let lp_coeffs = LPCoefficients {
            values: [Q15::from_f32(-0.5), Q15::ZERO, Q15::ZERO, Q15::ZERO, Q15::ZERO,
                     Q15::ZERO, Q15::ZERO, Q15::ZERO, Q15::ZERO, Q15::ZERO],
            reflection_coeffs: [Q15::ZERO; LP_ORDER],
        };
        
        let weighted = filter.create_filter(&lp_coeffs);
        let h = filter.compute_impulse_response(&weighted);
        
        // First sample should be 1 (unit impulse)
        assert_eq!(h[0], Q15::ONE);
        
        // Response should decay
        assert!(h[1].0.abs() < h[0].0.abs());
    }

    #[test]
    fn test_filter_signal() {
        let filter = PerceptualWeightingFilter::new();
        
        let lp_coeffs = LPCoefficients {
            values: [Q15::from_f32(-0.9), Q15::ZERO, Q15::ZERO, Q15::ZERO, Q15::ZERO,
                     Q15::ZERO, Q15::ZERO, Q15::ZERO, Q15::ZERO, Q15::ZERO],
            reflection_coeffs: [Q15::ZERO; LP_ORDER],
        };
        
        let weighted = filter.create_filter(&lp_coeffs);
        
        // Test with impulse
        let signal = vec![Q15::ONE, Q15::ZERO, Q15::ZERO, Q15::ZERO];
        let output = filter.filter_signal(&signal, &weighted);
        
        // First output should be close to input
        assert!((output[0].to_f32() - 1.0).abs() < 0.1);
        
        // Should have some response due to pole
        assert!(output[1].0 != 0);
    }

    #[test]
    fn test_bandwidth_expansion() {
        let filter = PerceptualWeightingFilter::new();
        
        let coeffs = [Q15::from_f32(0.8), Q15::from_f32(-0.4), Q15::from_f32(0.2),
                      Q15::ZERO, Q15::ZERO, Q15::ZERO, Q15::ZERO, Q15::ZERO, 
                      Q15::ZERO, Q15::ZERO];
        
        let expanded = filter.apply_bandwidth_expansion(&coeffs, Q15::from_f32(0.9));
        
        // Check values
        assert!((expanded[0].to_f32() - 0.72).abs() < 0.01);  // 0.8 * 0.9
        assert!((expanded[1].to_f32() + 0.324).abs() < 0.01); // -0.4 * 0.9^2
        assert!((expanded[2].to_f32() - 0.146).abs() < 0.02); // 0.2 * 0.9^3
    }
} 