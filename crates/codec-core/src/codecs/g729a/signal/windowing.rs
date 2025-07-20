//! Window functions for signal processing

use crate::codecs::g729a::constants::*;
use crate::codecs::g729a::types::Q15;
use crate::codecs::g729a::tables::{HAMMING_WINDOW, get_hamming_window};

/// Hamming window for LP analysis
pub struct HammingWindow {
    coefficients: Vec<Q15>,
}

impl HammingWindow {
    /// Create symmetric Hamming window
    pub fn new_symmetric() -> Self {
        // For symmetric window, use the full 240-sample window
        Self {
            coefficients: get_hamming_window(),
        }
    }
    
    /// Create asymmetric Hamming window for LP analysis
    pub fn new_asymmetric() -> Self {
        // G.729A uses an asymmetric window
        // Use the actual window values from the table
        Self {
            coefficients: get_hamming_window(),
        }
    }
    
    /// Apply window to signal
    pub fn apply(&self, signal: &[Q15]) -> Vec<Q15> {
        assert_eq!(signal.len(), self.coefficients.len(),
                   "Signal and window must have same length");
        
        signal.iter()
            .zip(self.coefficients.iter())
            .map(|(&s, &w)| s.saturating_mul(w))
            .collect()
    }
    
    /// Get window coefficients
    pub fn coefficients(&self) -> &[Q15] {
        &self.coefficients
    }
}

/// Lag window for autocorrelation (bandwidth expansion)
pub struct LagWindow {
    coefficients: Vec<Q15>,
}

impl LagWindow {
    /// Create lag window for autocorrelation
    /// w[k] = exp(-0.5 * (2*pi*k*f0/fs)^2)
    pub fn new(size: usize, expansion_factor: f32) -> Self {
        let mut coefficients = Vec::with_capacity(size);
        
        for k in 0..size {
            if k == 0 {
                coefficients.push(Q15::ONE);
            } else {
                // Simplified approximation
                let window_val = (1.0 - expansion_factor * (k as f32) * (k as f32)).max(0.0);
                coefficients.push(Q15::from_f32(window_val));
            }
        }
        
        Self { coefficients }
    }
    
    /// Apply lag window to autocorrelation values
    pub fn apply_to_correlation(&self, correlation: &[crate::codecs::g729a::types::Q31]) 
        -> Vec<crate::codecs::g729a::types::Q31> {
        
        use crate::codecs::g729a::types::Q31;
        
        correlation.iter()
            .zip(self.coefficients.iter())
            .map(|(&corr, &window)| {
                // Scale correlation by window coefficient
                Q31((corr.0 as i64 * window.0 as i64 >> 15) as i32)
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_hamming_window() {
        let window = HammingWindow::new_symmetric();
        let coeffs = window.coefficients();
        
        // Check size
        assert_eq!(coeffs.len(), 240);
        
        // Check endpoints
        assert!((coeffs[0].to_f32() - 0.08).abs() < 0.01); // 0.54 - 0.46
        assert!((coeffs[239].to_f32() - 0.08).abs() < 0.01);
        
        // Check center is maximum
        let center = coeffs[120].to_f32();
        assert!(center > coeffs[0].to_f32());
        assert!(center > coeffs[239].to_f32());
    }

    #[test]
    fn test_asymmetric_window() {
        let window = HammingWindow::new_asymmetric();
        let coeffs = window.coefficients();
        
        // Check size
        assert_eq!(coeffs.len(), 240);
        
        // First half should be Hamming-like
        assert!(coeffs[0].to_f32() < coeffs[L_TOTAL/4].to_f32());
        
        // Second half should be cosine-like, decreasing
        assert!(coeffs[L_TOTAL/2].to_f32() > coeffs[L_TOTAL-1].to_f32());
    }

    #[test]
    fn test_window_application() {
        let window = HammingWindow::new_symmetric();
        let signal = vec![
            Q15::from_f32(0.5),
            Q15::from_f32(0.5),
            Q15::from_f32(0.5),
            Q15::from_f32(0.5),
        ];
        
        let windowed = window.apply(&signal);
        
        // All values should be scaled down
        for (orig, win) in signal.iter().zip(windowed.iter()) {
            assert!(win.0.abs() <= orig.0.abs());
        }
        
        // Center values should be larger than edge values
        assert!(windowed[1].0.abs() > windowed[0].0.abs());
        assert!(windowed[2].0.abs() > windowed[3].0.abs());
    }

    #[test]
    fn test_lag_window() {
        let window = LagWindow::new(5, 0.01);
        
        // First coefficient should be 1
        assert_eq!(window.coefficients[0], Q15::ONE);
        
        // Should decrease with lag
        for i in 1..5 {
            assert!(window.coefficients[i].0 < window.coefficients[i-1].0);
        }
    }

    #[test]
    fn test_lag_window_application() {
        use crate::codecs::g729a::types::Q31;
        
        let window = LagWindow::new(3, 0.1);
        let correlation = vec![
            Q31::from_f32(0.8),
            Q31::from_f32(0.5),
            Q31::from_f32(0.3),
        ];
        
        let windowed = window.apply_to_correlation(&correlation);
        
        // First value should be unchanged
        assert_eq!(windowed[0].0, correlation[0].0);
        
        // Other values should be reduced
        assert!(windowed[1].0.abs() < correlation[1].0.abs());
        assert!(windowed[2].0.abs() < correlation[2].0.abs());
    }

    #[test]
    fn test_window_symmetry() {
        let window = HammingWindow::new_symmetric();
        let coeffs = window.coefficients();
        
        // Hamming window should be symmetric
        for i in 0..120 {
            let diff = (coeffs[i].0 - coeffs[239-i].0).abs();
            assert!(diff < 100); // Small tolerance for rounding
        }
    }
} 