//! Post-processing and filtering for synthesized speech

use crate::codecs::g729a::constants::*;
use crate::codecs::g729a::types::{Q15, Q31, LPCoefficients};
use crate::codecs::g729a::math::{FixedPointOps, energy};
// HighPassFilter is defined below in this file

/// Adaptive postfilter for G.729A
pub struct AdaptivePostfilter {
    /// Long-term (pitch) postfilter state
    pitch_delay_buffer: Vec<Q15>,
    
    /// Short-term (formant) postfilter memory
    formant_mem: Vec<Q15>,
    residual_mem: Vec<Q15>,
    
    /// Gain control
    past_gain: Q15,
    
    /// High-pass filter for output
    hp_filter: HighPassFilter,
}

impl AdaptivePostfilter {
    /// Create a new adaptive postfilter
    pub fn new() -> Self {
        Self {
            pitch_delay_buffer: vec![Q15::ZERO; PIT_MAX as usize + SUBFRAME_SIZE],
            formant_mem: vec![Q15::ZERO; LP_ORDER],
            residual_mem: vec![Q15::ZERO; LP_ORDER],
            past_gain: Q15::ONE,
            hp_filter: HighPassFilter::new(),
        }
    }
    
    /// Process synthesized speech through adaptive postfilter
    pub fn process(
        &mut self,
        synthesized: &[Q15],
        lp_coeffs: &LPCoefficients,
        pitch_delay: f32,
    ) -> Vec<Q15> {
        let mut output = Vec::with_capacity(synthesized.len());
        
        #[cfg(debug_assertions)]
        {
            let input_energy: i64 = synthesized.iter().map(|&x| (x.0 as i64).pow(2)).sum();
            eprintln!("Postfilter input energy: {}", input_energy);
        }
        
        // Process in subframes
        for i in 0..2 {
            let start = i * SUBFRAME_SIZE;
            let end = start + SUBFRAME_SIZE;
            let subframe = &synthesized[start..end];
            
            // 1. Long-term postfiltering (pitch enhancement)
            let pitch_enhanced = self.long_term_postfilter(subframe, pitch_delay);
            
            #[cfg(debug_assertions)]
            {
                let pitch_energy: i64 = pitch_enhanced.iter().map(|&x| (x.0 as i64).pow(2)).sum();
                eprintln!("  After pitch filter: {}", pitch_energy);
            }
            
            // 2. Short-term postfiltering (formant enhancement)
            let formant_enhanced = self.short_term_postfilter(&pitch_enhanced, lp_coeffs);
            
            #[cfg(debug_assertions)]
            {
                let formant_energy: i64 = formant_enhanced.iter().map(|&x| (x.0 as i64).pow(2)).sum();
                eprintln!("  After formant filter: {}", formant_energy);
            }
            
            // 3. Gain control to match original energy
            let gain_controlled = self.apply_gain_control(&formant_enhanced, subframe);
            
            #[cfg(debug_assertions)]
            {
                let gain_energy: i64 = gain_controlled.iter().map(|&x| (x.0 as i64).pow(2)).sum();
                eprintln!("  After gain control: {}", gain_energy);
            }
            
            output.extend_from_slice(&gain_controlled);
        }
        
        #[cfg(debug_assertions)]
        {
            let pre_hp_energy: i64 = output.iter().map(|&x| (x.0 as i64).pow(2)).sum();
            eprintln!("Before high-pass filter: {}", pre_hp_energy);
        }
        
        // 4. High-pass filtering
        let final_output = self.hp_filter.filter(&output);
        
        #[cfg(debug_assertions)]
        {
            let final_energy: i64 = final_output.iter().map(|&x| (x.0 as i64).pow(2)).sum();
            eprintln!("Final postfilter output energy: {}", final_energy);
        }
        
        final_output
    }
    
    /// Long-term postfilter for pitch enhancement
    fn long_term_postfilter(&mut self, signal: &[Q15], pitch_delay: f32) -> Vec<Q15> {
        let mut output = vec![Q15::ZERO; SUBFRAME_SIZE];
        
        // Update delay buffer
        self.pitch_delay_buffer.extend_from_slice(signal);
        if self.pitch_delay_buffer.len() > PIT_MAX as usize + SUBFRAME_SIZE {
            self.pitch_delay_buffer.drain(0..SUBFRAME_SIZE);
        }
        
        // Compute optimal gain for pitch postfilter
        let int_delay = pitch_delay.round() as usize;
        let gain = self.compute_pitch_gain(signal, int_delay);
        
        // Apply pitch postfilter: H(z) = 1/(1 - g*z^(-T))
        // Simplified to: y[n] = x[n] + g * y[n-T]
        for n in 0..SUBFRAME_SIZE {
            let delayed_idx = self.pitch_delay_buffer.len() - int_delay - SUBFRAME_SIZE + n;
            if delayed_idx < self.pitch_delay_buffer.len() {
                let delayed = self.pitch_delay_buffer[delayed_idx];
                let contribution = delayed.saturating_mul(gain);
                output[n] = signal[n].saturating_add(contribution);
            } else {
                output[n] = signal[n];
            }
        }
        
        output
    }
    
    /// Compute optimal pitch postfilter gain
    fn compute_pitch_gain(&self, signal: &[Q15], delay: usize) -> Q15 {
        if delay == 0 || delay >= self.pitch_delay_buffer.len() {
            return Q15::ZERO;
        }
        
        let mut correlation = Q31::ZERO;
        let mut energy_delayed = Q31::ZERO;
        
        let buffer_start = self.pitch_delay_buffer.len() - delay - SUBFRAME_SIZE;
        
        for i in 0..SUBFRAME_SIZE {
            if buffer_start + i < self.pitch_delay_buffer.len() {
                let delayed = self.pitch_delay_buffer[buffer_start + i];
                let prod = signal[i].to_q31().saturating_mul(delayed.to_q31());
                correlation = correlation.saturating_add(prod);
                
                let energy_term = delayed.to_q31().saturating_mul(delayed.to_q31());
                energy_delayed = energy_delayed.saturating_add(energy_term);
            }
        }
        
        // Gain = correlation / energy, limited to [0, 0.5]
        if energy_delayed.0 > 0 {
            let gain = Q15((correlation.0 / (energy_delayed.0 >> 15).max(1)) as i16);
            // Limit gain to 0.5
            Q15(gain.0.min(Q15_ONE / 2))
        } else {
            Q15::ZERO
        }
    }
    
    /// Short-term postfilter for formant enhancement
    fn short_term_postfilter(
        &mut self,
        signal: &[Q15],
        lp_coeffs: &LPCoefficients,
    ) -> Vec<Q15> {
        // Postfilter: H(z) = A(z/γn) / A(z/γd)
        // γn = 0.55, γd = 0.7 for G.729A
        const GAMMA_NUM: Q15 = Q15(18022);  // 0.55 in Q15
        const GAMMA_DEN: Q15 = Q15(22938);  // 0.7 in Q15
        
        let mut output = vec![Q15::ZERO; signal.len()];
        
        // Apply numerator filter A(z/γn) - zeros
        let mut residual = vec![Q15::ZERO; signal.len()];
        for n in 0..signal.len() {
            let mut sum = signal[n].to_q31();
            
            // Apply LP coefficients with bandwidth expansion
            let mut gamma_power = GAMMA_NUM;
            for k in 0..LP_ORDER.min(n) {
                let coeff = lp_coeffs.values[k].saturating_mul(gamma_power);
                let prod = coeff.to_q31().saturating_mul(signal[n - k - 1].to_q31());
                sum = sum.saturating_add(prod);
                gamma_power = gamma_power.saturating_mul(GAMMA_NUM);
            }
            
            residual[n] = sum.to_q15();
        }
        
        // Apply denominator filter 1/A(z/γd) - poles
        for n in 0..residual.len() {
            let mut sum = residual[n].to_q31();
            
            // Use past output values
            let mut gamma_power = GAMMA_DEN;
            for k in 0..LP_ORDER.min(n) {
                let coeff = lp_coeffs.values[k].saturating_mul(gamma_power);
                let mem_idx = if n > k { n - k - 1 } else { 0 };
                let prod = coeff.to_q31().saturating_mul(output[mem_idx].to_q31());
                sum = sum.saturating_add(Q31(-prod.0));
                gamma_power = gamma_power.saturating_mul(GAMMA_DEN);
            }
            
            output[n] = sum.to_q15();
        }
        
        // Update memory for next subframe
        if output.len() >= LP_ORDER {
            self.formant_mem.copy_from_slice(&output[output.len() - LP_ORDER..]);
        }
        
        output
    }
    
    /// Apply automatic gain control
    fn apply_gain_control(&mut self, filtered: &[Q15], original: &[Q15]) -> Vec<Q15> {
        // Compute energies
        let energy_filtered = energy(filtered);
        let energy_original = energy(original);
        
        if energy_filtered.0 <= 0 || energy_original.0 <= 0 {
            return filtered.to_vec();
        }
        
        // Compute gain factor = sqrt(E_original / E_filtered)
        let ratio = Q31((energy_original.0 as i64 * ((1i64 << 31) - 1) / energy_filtered.0.max(1) as i64) as i32);
        let gain = self.approximate_sqrt(ratio).to_q15();
        
        // Smooth gain with past value
        let alpha = Q15(29491); // 0.9 in Q15
        let smoothed_gain = self.past_gain.saturating_mul(alpha)
            .saturating_add(gain.saturating_mul(Q15::ONE.saturating_add(Q15(-alpha.0))));
        
        self.past_gain = smoothed_gain;
        
        // Apply gain
        let mut output = vec![Q15::ZERO; filtered.len()];
        for i in 0..filtered.len() {
            output[i] = filtered[i].saturating_mul(smoothed_gain);
        }
        
        output
    }
    
    /// Approximate square root for gain control
    fn approximate_sqrt(&self, x: Q31) -> Q31 {
        if x.0 <= 0 {
            return Q31::ZERO;
        }
        
        // Simple approximation using bit shifts
        let mut val = x.0 as u32;
        let mut shift = 0;
        
        // Normalize to range [0.25, 1.0]
        while val < 0x10000000 {
            val <<= 2;
            shift += 1;
        }
        
        // Linear approximation in normalized range
        // sqrt(x) ≈ 0.5 + 0.5*x for x in [0.25, 1.0]
        let normalized = Q31(val as i32);
        let q31_half = 1i32 << 30; // 0.5 in Q31
        let result = Q31(q31_half).saturating_add(
            Q31((normalized.0 as i64 * q31_half as i64 >> 31) as i32)
        );
        
        // Denormalize
        Q31(result.0 >> shift)
    }
    
    /// Reset postfilter state
    pub fn reset(&mut self) {
        self.pitch_delay_buffer.fill(Q15::ZERO);
        self.formant_mem.fill(Q15::ZERO);
        self.residual_mem.fill(Q15::ZERO);
        self.past_gain = Q15::ONE;
        self.hp_filter = HighPassFilter::new();
    }
}

/// High-pass filter for final output
pub struct HighPassFilter {
    /// Input delay line
    x_state: [Q15; 2],
    /// Output delay line  
    y_state: [Q15; 2],
}

impl HighPassFilter {
    /// Create new high-pass filter
    pub fn new() -> Self {
        Self {
            x_state: [Q15::ZERO; 2],
            y_state: [Q15::ZERO; 2],
        }
    }
    
    /// Apply high-pass filtering
    /// H(z) = 0.46363718 - 0.92724705*z^-1 + 0.46363718*z^-2
    ///        -------------------------------------------
    ///        1 - 1.9059465*z^-1 + 0.9114024*z^-2
    pub fn filter(&mut self, input: &[Q15]) -> Vec<Q15> {
        // Filter coefficients in Q15
        const B0: Q15 = Q15(15183);  // 0.46363718
        const B1: Q15 = Q15(-30367); // -0.92724705
        const B2: Q15 = Q15(15183);  // 0.46363718
        const A1: Q15 = Q15(31259);  // 1.9059465 (positive for subtraction)
        const A2: Q15 = Q15(-29837); // -0.9114024 (negative for subtraction)
        
        let mut output = vec![Q15::ZERO; input.len()];
        
        for i in 0..input.len() {
            let x = input[i];
            
            // Compute numerator: b0*x[n] + b1*x[n-1] + b2*x[n-2]
            let mut y = B0.to_q31().saturating_mul(x.to_q31());
            y = y.saturating_add(B1.to_q31().saturating_mul(self.x_state[0].to_q31()));
            y = y.saturating_add(B2.to_q31().saturating_mul(self.x_state[1].to_q31()));
            
            // Subtract denominator: - a1*y[n-1] - a2*y[n-2] 
            y = y.saturating_add(A1.to_q31().saturating_mul(self.y_state[0].to_q31()));
            y = y.saturating_add(A2.to_q31().saturating_mul(self.y_state[1].to_q31()));
            
            let y_out = y.to_q15();
            output[i] = y_out;
            
            // Update delay lines
            self.x_state[1] = self.x_state[0];
            self.x_state[0] = x;
            self.y_state[1] = self.y_state[0];
            self.y_state[0] = y_out;
        }
        
        output
    }
}

impl Default for AdaptivePostfilter {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_postfilter_creation() {
        let postfilter = AdaptivePostfilter::new();
        assert_eq!(postfilter.past_gain, Q15::ONE);
    }

    #[test]
    fn test_high_pass_filter() {
        let mut hpf = HighPassFilter::new();
        
        // Test with DC input
        let dc_input = vec![Q15::from_f32(0.5); 100];
        let output = hpf.filter(&dc_input);
        
        // After initial transient, output should be near zero
        let last_samples = &output[90..];
        let avg: i32 = last_samples.iter().map(|&x| x.0 as i32).sum::<i32>() / 10;
        assert!(avg.abs() < 100); // Should remove DC
    }

    #[test]
    fn test_gain_control() {
        let mut postfilter = AdaptivePostfilter::new();
        
        // Test with reduced energy signal
        let original = vec![Q15::from_f32(0.5); SUBFRAME_SIZE];
        let filtered = vec![Q15::from_f32(0.25); SUBFRAME_SIZE]; // Half energy
        
        let controlled = postfilter.apply_gain_control(&filtered, &original);
        
        // Output energy should be closer to original
        let energy_out = energy(&controlled);
        let energy_orig = energy(&original);
        
        let ratio = (energy_out.0 as f64 / energy_orig.0 as f64);
        assert!(ratio > 0.8 && ratio < 1.2); // Within 20% of original
    }
} 