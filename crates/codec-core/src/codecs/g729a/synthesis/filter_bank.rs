//! Synthesis filter bank for G.729A

use crate::codecs::g729a::constants::*;
use crate::codecs::g729a::types::{Q15, Q31, LPCoefficients};
use crate::codecs::g729a::math::FixedPointOps;

/// Synthesis filter state
pub struct SynthesisFilter {
    /// Filter memory (past outputs)
    memory: [Q15; LP_ORDER],
}

impl SynthesisFilter {
    /// Create a new synthesis filter
    pub fn new() -> Self {
        Self {
            memory: [Q15::ZERO; LP_ORDER],
        }
    }
    
    /// Synthesize speech from excitation using LP coefficients
    /// y[n] = e[n] - Σ(a[k] * y[n-k]) for k=1..LP_ORDER
    pub fn synthesize(&mut self, excitation: &[Q15], lp_coeffs: &[Q15]) -> Vec<Q15> {
        let mut output = Vec::with_capacity(excitation.len());
        
        for &exc_sample in excitation {
            // Compute filter output
            let mut sum = exc_sample.to_q31();
            
            for k in 0..LP_ORDER {
                let coeff = lp_coeffs[k];
                let past_output = self.memory[k];
                let prod = coeff.to_q31().saturating_mul(past_output.to_q31());
                sum = sum.saturating_add(Q31(-prod.0));
            }
            
            let output_sample = sum.to_q15();
            output.push(output_sample);
            
            // Update memory
            for i in (1..LP_ORDER).rev() {
                self.memory[i] = self.memory[i - 1];
            }
            self.memory[0] = output_sample;
        }
        
        output
    }
    
    /// Reset filter memory
    pub fn reset(&mut self) {
        self.memory = [Q15::ZERO; LP_ORDER];
    }
    
    /// Get current filter memory
    pub fn get_memory(&self) -> &[Q15; LP_ORDER] {
        &self.memory
    }
    
    /// Set filter memory
    pub fn set_memory(&mut self, memory: [Q15; LP_ORDER]) {
        self.memory = memory;
    }
}

/// Interpolating synthesis filter for smooth transitions
pub struct InterpolatingSynthesisFilter {
    /// Main synthesis filter
    filter: SynthesisFilter,
}

impl InterpolatingSynthesisFilter {
    /// Create a new interpolating synthesis filter
    pub fn new() -> Self {
        Self {
            filter: SynthesisFilter::new(),
        }
    }
    
    /// Synthesize with interpolated LP coefficients
    pub fn synthesize_interpolated(
        &mut self,
        excitation: &[Q15],
        lp_start: &[Q15],
        lp_end: &[Q15],
        interpolation_steps: usize,
    ) -> Vec<Q15> {
        let mut output = Vec::with_capacity(excitation.len());
        let exc_per_step = excitation.len() / interpolation_steps;
        
        for step in 0..interpolation_steps {
            // Compute interpolation weight
            let weight = Q15::from_f32(step as f32 / interpolation_steps as f32);
            
            // Interpolate LP coefficients
            let mut interpolated_lp = [Q15::ZERO; LP_ORDER];
            for i in 0..LP_ORDER {
                let start_weighted = lp_start[i].saturating_mul(Q15::ONE.saturating_add(Q15(-weight.0)));
                let end_weighted = lp_end[i].saturating_mul(weight);
                interpolated_lp[i] = start_weighted.saturating_add(end_weighted);
            }
            
            // Synthesize this segment
            let start_idx = step * exc_per_step;
            let end_idx = ((step + 1) * exc_per_step).min(excitation.len());
            let segment_output = self.filter.synthesize(
                &excitation[start_idx..end_idx],
                &interpolated_lp,
            );
            
            output.extend_from_slice(&segment_output);
        }
        
        output
    }
    
    /// Reset filter state
    pub fn reset(&mut self) {
        self.filter.reset();
    }
}

impl Default for SynthesisFilter {
    fn default() -> Self {
        Self::new()
    }
}

impl Default for InterpolatingSynthesisFilter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_synthesis_filter_creation() {
        let filter = SynthesisFilter::new();
        assert_eq!(filter.memory, [Q15::ZERO; LP_ORDER]);
    }

    #[test]
    fn test_synthesis_filter_impulse() {
        let mut filter = SynthesisFilter::new();
        
        // Test with unit impulse and simple AR coefficients
        let excitation = vec![Q15::ONE, Q15::ZERO, Q15::ZERO, Q15::ZERO];
        let lp_coeffs = [Q15::from_f32(-0.5), Q15::ZERO, Q15::ZERO, Q15::ZERO,
                         Q15::ZERO, Q15::ZERO, Q15::ZERO, Q15::ZERO,
                         Q15::ZERO, Q15::ZERO];
        
        let output = filter.synthesize(&excitation, &lp_coeffs);
        
        // First output should be 1 (impulse)
        assert_eq!(output[0], Q15::ONE);
        
        // Second output should be 0.5 (due to feedback)
        assert!((output[1].to_f32() - 0.5).abs() < 0.01);
    }

    #[test]
    fn test_memory_management() {
        let mut filter = SynthesisFilter::new();
        
        // Set custom memory
        let memory = [Q15::from_f32(0.1); LP_ORDER];
        filter.set_memory(memory);
        
        // Check it was set
        assert_eq!(filter.get_memory(), &memory);
        
        // Reset
        filter.reset();
        assert_eq!(filter.get_memory(), &[Q15::ZERO; LP_ORDER]);
    }

    #[test]
    fn test_interpolating_filter() {
        let mut filter = InterpolatingSynthesisFilter::new();
        
        // Test with simple excitation
        let excitation = vec![Q15::from_f32(0.1); 20];
        
        // Start and end LP coefficients
        let lp_start = [Q15::from_f32(-0.8), Q15::ZERO, Q15::ZERO, Q15::ZERO,
                        Q15::ZERO, Q15::ZERO, Q15::ZERO, Q15::ZERO,
                        Q15::ZERO, Q15::ZERO];
        
        let lp_end = [Q15::from_f32(-0.4), Q15::ZERO, Q15::ZERO, Q15::ZERO,
                      Q15::ZERO, Q15::ZERO, Q15::ZERO, Q15::ZERO,
                      Q15::ZERO, Q15::ZERO];
        
        let output = filter.synthesize_interpolated(
            &excitation,
            &lp_start,
            &lp_end,
            4, // 4 interpolation steps
        );
        
        assert_eq!(output.len(), excitation.len());
    }

    #[test]
    fn test_synthesis_stability() {
        let mut filter = SynthesisFilter::new();
        
        // Test with stable coefficients
        let excitation = vec![Q15::from_f32(0.01); 100];
        let lp_coeffs = [Q15::from_f32(-0.9), Q15::from_f32(0.1), Q15::ZERO, Q15::ZERO,
                         Q15::ZERO, Q15::ZERO, Q15::ZERO, Q15::ZERO,
                         Q15::ZERO, Q15::ZERO];
        
        let output = filter.synthesize(&excitation, &lp_coeffs);
        
        // Check that output doesn't blow up
        for sample in &output {
            assert!(sample.0.abs() < Q15_ONE);
        }
    }
} 