//! Linear prediction analysis using Levinson-Durbin algorithm

use crate::codecs::g729a::constants::*;
use crate::codecs::g729a::types::{Q15, Q31, LPCoefficients};
use crate::codecs::g729a::math::{autocorrelation, FixedPointOps};
use crate::codecs::g729a::signal::{HammingWindow, LagWindow};

/// Linear predictor for spectral analysis
pub struct LinearPredictor {
    /// Window for autocorrelation
    autocorr_window: HammingWindow,
    /// Lag window for stability
    lag_window: LagWindow,
}

impl LinearPredictor {
    /// Create a new linear predictor
    pub fn new() -> Self {
        Self {
            autocorr_window: HammingWindow::new_asymmetric(),
            lag_window: LagWindow::new(LP_ORDER + 1, 0.0001),
        }
    }
    
    /// Analyze windowed signal and extract LP coefficients
    pub fn analyze(&self, windowed_signal: &[Q15]) -> LPCoefficients {
        // 1. Compute autocorrelation
        let correlations = autocorrelation(windowed_signal, LP_ORDER + 1);
        
        #[cfg(debug_assertions)]
        {
            eprintln!("Autocorrelation debug:");
            eprintln!("  R[0] (energy): {}", correlations[0].0);
            eprintln!("  R[1..5]: {:?}", &correlations[1..6].iter().map(|x| x.0).collect::<Vec<_>>());
        }
        
        // 2. Apply lag windowing for numerical stability
        let windowed_corr = self.lag_window.apply_to_correlation(&correlations);
        
        #[cfg(debug_assertions)]
        {
            eprintln!("After lag windowing:");
            eprintln!("  R[0]: {}, R[1]: {}", windowed_corr[0].0, windowed_corr[1].0);
        }
        
        // 3. Levinson-Durbin recursion
        let (coefficients, reflection_coeffs) = self.levinson_durbin(&windowed_corr);
        
        #[cfg(debug_assertions)]
        {
            eprintln!("LP coefficients: {:?}", coefficients.iter().map(|x| x.0).collect::<Vec<_>>());
        }
        
        // 4. Bandwidth expansion (done in quantization stage for G.729A)
        LPCoefficients {
            values: coefficients,
            reflection_coeffs,
        }
    }
    
    /// Levinson-Durbin algorithm to solve Toeplitz system
    fn levinson_durbin(&self, r: &[Q31]) -> ([Q15; LP_ORDER], [Q15; LP_ORDER]) {
        let mut a = [[Q15::ZERO; LP_ORDER + 1]; LP_ORDER + 1];
        let mut k = [Q15::ZERO; LP_ORDER];
        
        // Check for zero energy
        if r[0].0 <= 0 {
            return ([Q15::ZERO; LP_ORDER], k);
        }
        
        // Initialize
        a[0][0] = Q15::ONE;
        let mut alpha = r[0];
        
        // Recursion
        for m in 1..=LP_ORDER {
            // Compute reflection coefficient
            let mut sum = Q31::ZERO;
            for j in 1..m {
                let prod = a[m-1][j].to_q31().saturating_mul(r[m-j].to_q15().to_q31());
                sum = sum.saturating_add(prod);
            }
            
            let numerator = r[m].saturating_add(sum);
            
            // Avoid division by zero
            if alpha.0 <= 0 {
                break;
            }
            
            // k[m] = -(r[m] + sum) / alpha
            // Approximate division using multiplication by inverse
            let k_m = Q15(-(numerator.0 / (alpha.0 >> 15).max(1)) as i16);
            k[m-1] = k_m;
            
            // Update coefficients
            for j in 1..=m/2 {
                let tmp = a[m-1][j].saturating_add(k_m.saturating_mul(a[m-1][m-j]));
                a[m][m-j] = a[m-1][m-j].saturating_add(k_m.saturating_mul(a[m-1][j]));
                a[m][j] = tmp;
            }
            
            a[m][m] = k_m;
            
            // Update prediction error
            let k_sq = k_m.saturating_mul(k_m);
            let factor = Q15::ONE.saturating_add(Q15(-k_sq.0));
            alpha = Q31((alpha.0 as i64 * factor.0 as i64 >> 15) as i32);
            
            // Check stability
            if k_m.0.abs() >= Q15_ONE - 100 {
                // Near instability, stop iteration
                break;
            }
        }
        
        // Extract final coefficients (skip a[0] = 1)
        let mut coeffs = [Q15::ZERO; LP_ORDER];
        for i in 0..LP_ORDER {
            coeffs[i] = a[LP_ORDER][i + 1];
        }
        
        (coeffs, k)
    }
    
    /// Apply bandwidth expansion to LP coefficients
    pub fn expand_bandwidth(coeffs: &[Q15], gamma: Q15) -> Vec<Q15> {
        let mut expanded = Vec::with_capacity(coeffs.len());
        let mut gamma_power = Q15::ONE;
        
        for &coeff in coeffs {
            expanded.push(coeff.saturating_mul(gamma_power));
            gamma_power = gamma_power.saturating_mul(gamma);
        }
        
        expanded
    }
}

impl Default for LinearPredictor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_linear_predictor_creation() {
        let predictor = LinearPredictor::new();
        // Just ensure it creates without panic
        assert!(true);
    }

    #[test]
    fn test_levinson_durbin_impulse() {
        let predictor = LinearPredictor::new();
        
        // Test with impulse autocorrelation (white noise)
        let r = vec![
            Q31::from_f32(0.5),
            Q31::ZERO,
            Q31::ZERO,
            Q31::ZERO,
        ];
        
        let (coeffs, k) = predictor.levinson_durbin(&r);
        
        // For white noise, all LP coefficients should be near zero
        for coeff in &coeffs[0..3] {
            assert!(coeff.0.abs() < 1000);
        }
        
        // All reflection coefficients should be near zero
        for reflection in &k[0..3] {
            assert!(reflection.0.abs() < 1000);
        }
    }

    #[test]
    fn test_levinson_durbin_correlated() {
        let predictor = LinearPredictor::new();
        
        // Test with correlated signal (AR process)
        let r = vec![
            Q31::from_f32(0.5),
            Q31::from_f32(0.4),
            Q31::from_f32(0.3),
            Q31::from_f32(0.2),
            Q31::from_f32(0.1),
        ];
        
        let (coeffs, k) = predictor.levinson_durbin(&r);
        
        // First coefficient should be negative (due to positive correlation)
        assert!(coeffs[0].0 < 0);
        
        // Reflection coefficients should be stable (|k| < 1)
        for reflection in &k[0..4] {
            assert!(reflection.0.abs() < Q15_ONE);
        }
    }

    #[test]
    fn test_bandwidth_expansion() {
        let coeffs = vec![
            Q15::from_f32(0.8),
            Q15::from_f32(0.4),
            Q15::from_f32(0.2),
        ];
        
        let gamma = Q15::from_f32(0.9);
        let expanded = LinearPredictor::expand_bandwidth(&coeffs, gamma);
        
        assert_eq!(expanded.len(), coeffs.len());
        
        // Check that magnitudes decrease
        assert!(expanded[0].0.abs() <= coeffs[0].0.abs());
        assert!(expanded[1].0.abs() <= coeffs[1].0.abs());
        assert!(expanded[2].0.abs() <= coeffs[2].0.abs());
        
        // Check approximate values
        assert!((expanded[0].to_f32() - 0.8).abs() < 0.01);
        assert!((expanded[1].to_f32() - 0.36).abs() < 0.02); // 0.4 * 0.9
        assert!((expanded[2].to_f32() - 0.162).abs() < 0.02); // 0.2 * 0.9^2
    }

    #[test]
    fn test_full_analysis() {
        let predictor = LinearPredictor::new();
        
        // Create a simple test signal
        let mut signal = vec![Q15::ZERO; WINDOW_SIZE];
        for i in 0..WINDOW_SIZE {
            let value = ((i as f32 / 10.0).sin() * 0.5) * (1.0 - i as f32 / WINDOW_SIZE as f32);
            signal[i] = Q15::from_f32(value);
        }
        
        // Apply window
        let windowed = predictor.autocorr_window.apply(&signal);
        
        // Analyze
        let lp_coeffs = predictor.analyze(&windowed);
        
        // Check that we got LP_ORDER coefficients
        assert_eq!(lp_coeffs.values.len(), LP_ORDER);
        assert_eq!(lp_coeffs.reflection_coeffs.len(), LP_ORDER);
        
        // All reflection coefficients should be stable
        for k in &lp_coeffs.reflection_coeffs {
            assert!(k.0.abs() < Q15_ONE);
        }
    }

    #[test]
    fn test_zero_input() {
        let predictor = LinearPredictor::new();
        
        // Test with zero input
        let r = vec![Q31::ZERO; LP_ORDER + 1];
        let (coeffs, k) = predictor.levinson_durbin(&r);
        
        // All coefficients should be zero
        for coeff in &coeffs {
            assert_eq!(coeff.0, 0);
        }
        for reflection in &k {
            assert_eq!(reflection.0, 0);
        }
    }
} 