//! Adaptive codebook (pitch predictor) for G.729A

use crate::codecs::g729a::constants::*;
use crate::codecs::g729a::types::{Q15, Q31};
use crate::codecs::g729a::math::{FixedPointOps, dot_product};
use crate::codecs::g729a::signal::backward_correlation;
use std::ops::Range;

/// Circular buffer for past excitation
pub struct CircularBuffer<T> {
    buffer: Vec<T>,
    size: usize,
    write_pos: usize,
}

impl<T: Copy + Default> CircularBuffer<T> {
    /// Create a new circular buffer
    pub fn new(size: usize) -> Self {
        Self {
            buffer: vec![T::default(); size],
            size,
            write_pos: 0,
        }
    }
    
    /// Write samples to buffer
    pub fn write(&mut self, samples: &[T]) {
        for &sample in samples {
            self.buffer[self.write_pos] = sample;
            self.write_pos = (self.write_pos + 1) % self.size;
        }
    }
    
    /// Read samples from buffer at a given delay
    pub fn read_at_delay(&self, delay: usize, length: usize) -> Vec<T> {
        let mut output = Vec::with_capacity(length);
        let start = (self.write_pos + self.size - delay) % self.size;
        
        for i in 0..length {
            let idx = (start + i) % self.size;
            output.push(self.buffer[idx]);
        }
        
        output
    }
    
    /// Get the most recent samples
    pub fn get_recent(&self, length: usize) -> Vec<T> {
        self.read_at_delay(length, length)
    }
}

/// Fractional delay filter for interpolation
pub struct FractionalDelayFilter;

impl FractionalDelayFilter {
    /// Interpolate signal with fractional delay
    /// Uses 6-tap Hamming windowed sinc filter
    pub fn interpolate(signal: &[Q15], delay: f32) -> Vec<Q15> {
        let int_delay = delay.floor() as usize;
        let frac_delay = delay - int_delay as f32;
        
        // Filter coefficients for different fractional delays
        let coeffs = Self::get_interpolation_coeffs(frac_delay);
        
        let mut output = Vec::with_capacity(SUBFRAME_SIZE);
        
        for n in 0..SUBFRAME_SIZE {
            let mut sum = Q31::ZERO;
            
            // 6-tap filter centered around the delay
            for k in 0..6 {
                let idx = (int_delay + n).saturating_sub(2) + k;
                if idx < signal.len() {
                    let coeff = coeffs[k];
                    let sample = signal[idx];
                    let prod = coeff.to_q31().saturating_mul(sample.to_q31());
                    sum = sum.saturating_add(prod);
                }
            }
            
            output.push(sum.to_q15());
        }
        
        output
    }
    
    /// Get interpolation filter coefficients for fractional delay
    fn get_interpolation_coeffs(frac: f32) -> [Q15; 6] {
        // Simplified coefficients for 1/3 resolution
        if frac < 0.167 {
            // frac ≈ 0
            [Q15::ZERO, Q15::ZERO, Q15::ZERO, Q15::ONE, Q15::ZERO, Q15::ZERO]
        } else if frac < 0.5 {
            // frac ≈ 1/3
            [Q15::from_f32(-0.05), Q15::from_f32(0.1), Q15::from_f32(-0.2),
             Q15::from_f32(0.85), Q15::from_f32(0.35), Q15::from_f32(-0.05)]
        } else {
            // frac ≈ 2/3
            [Q15::from_f32(-0.05), Q15::from_f32(0.35), Q15::from_f32(0.85),
             Q15::from_f32(-0.2), Q15::from_f32(0.1), Q15::from_f32(-0.05)]
        }
    }
}

/// Adaptive codebook contribution
#[derive(Debug, Clone)]
pub struct AdaptiveContribution {
    pub delay: f32,
    pub vector: [Q15; SUBFRAME_SIZE],
}

/// Adaptive codebook for pitch prediction
pub struct AdaptiveCodebook {
    /// Past excitation buffer
    past_excitation: CircularBuffer<Q15>,
    /// Fractional delay interpolator
    interpolation_filter: FractionalDelayFilter,
}

impl AdaptiveCodebook {
    /// Create a new adaptive codebook
    pub fn new() -> Self {
        Self {
            past_excitation: CircularBuffer::new(PIT_MAX as usize + SUBFRAME_SIZE),
            interpolation_filter: FractionalDelayFilter,
        }
    }
    
    /// Search for best pitch delay
    /// For G.729A: simplified search maximizing correlation only
    pub fn search(&mut self, target: &[Q15], pitch_range: &Range<f32>) -> AdaptiveContribution {
        let mut best_delay = pitch_range.start;
        let mut best_corr = Q31(i32::MIN);
        let mut best_vector = [Q15::ZERO; SUBFRAME_SIZE];
        
        // Get recent excitation for interpolation
        let recent = self.past_excitation.get_recent(PIT_MAX as usize + 10);
        
        // Search with 1/3 fractional resolution for delays < 85
        let step = if pitch_range.end < 85.0 { 0.333 } else { 1.0 };
        
        let mut delay = pitch_range.start;
        while delay < pitch_range.end {
            // Generate excitation at this delay
            let excitation = FractionalDelayFilter::interpolate(&recent, delay);
            
            // Compute correlation with target
            let corr = dot_product(&target[..SUBFRAME_SIZE], &excitation[..SUBFRAME_SIZE]);
            
            if corr.0 > best_corr.0 {
                best_corr = corr;
                best_delay = delay;
                best_vector.copy_from_slice(&excitation[..SUBFRAME_SIZE]);
            }
            
            delay += step;
        }
        
        AdaptiveContribution {
            delay: best_delay,
            vector: best_vector,
        }
    }
    
    /// Update excitation buffer with new samples
    pub fn update_excitation(&mut self, excitation: &[Q15]) {
        self.past_excitation.write(excitation);
    }
    
    /// Decode excitation vector from pitch delay
    pub fn decode_vector(&self, delay: f32) -> [Q15; SUBFRAME_SIZE] {
        let recent = self.past_excitation.get_recent(PIT_MAX as usize + 10);
        let excitation = FractionalDelayFilter::interpolate(&recent, delay);
        
        let mut vector = [Q15::ZERO; SUBFRAME_SIZE];
        vector.copy_from_slice(&excitation[..SUBFRAME_SIZE]);
        vector
    }
    
    /// Copy residual to excitation buffer (for delays < SUBFRAME_SIZE)
    pub fn copy_residual(&mut self, residual: &[Q15]) {
        // In G.729A, for short delays, the LP residual is used
        // to fill unknown samples
        self.past_excitation.write(residual);
    }
}

impl Default for AdaptiveCodebook {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_circular_buffer() {
        let mut buffer: CircularBuffer<i32> = CircularBuffer::new(5);
        
        // Write some values
        buffer.write(&[1, 2, 3, 4, 5]);
        
        // Read at delay 0 (most recent)
        let recent = buffer.read_at_delay(1, 1);
        assert_eq!(recent[0], 5);
        
        // Read at delay 2
        let delayed = buffer.read_at_delay(3, 3);
        assert_eq!(delayed, vec![3, 4, 5]);
        
        // Write more (wrap around)
        buffer.write(&[6, 7]);
        let recent = buffer.read_at_delay(1, 1);
        assert_eq!(recent[0], 7);
    }

    #[test]
    fn test_fractional_delay_filter() {
        // Test with unit impulse
        let mut signal = vec![Q15::ZERO; 10];
        signal[5] = Q15::ONE;
        
        // Test integer delay
        let delayed = FractionalDelayFilter::interpolate(&signal, 0.0);
        // Should have peak at position 3 (5 - 2 for filter center)
        assert!(delayed[3].0.abs() > 20000);
        
        // Test fractional delay
        let delayed = FractionalDelayFilter::interpolate(&signal, 0.333);
        // Should have spread energy
        assert!(delayed[2].0.abs() > 0 || delayed[3].0.abs() > 0 || delayed[4].0.abs() > 0);
    }

    #[test]
    fn test_adaptive_codebook_creation() {
        let codebook = AdaptiveCodebook::new();
        // Just ensure it creates without panic
        assert!(true);
    }

    #[test]
    fn test_adaptive_contribution() {
        let contrib = AdaptiveContribution {
            delay: 50.333,
            vector: [Q15::from_f32(0.1); SUBFRAME_SIZE],
        };
        
        assert_eq!(contrib.delay, 50.333);
        assert_eq!(contrib.vector[0], Q15::from_f32(0.1));
    }

    #[test]
    fn test_adaptive_search_simple() {
        let mut codebook = AdaptiveCodebook::new();
        
        // Initialize with some excitation
        let init_excitation = vec![Q15::from_f32(0.5); PIT_MAX as usize + 10];
        codebook.update_excitation(&init_excitation);
        
        // Create target that matches past excitation
        let target = vec![Q15::from_f32(0.5); SUBFRAME_SIZE];
        
        // Search in a small range
        let range = 40.0..50.0;
        let result = codebook.search(&target, &range);
        
        // Should find some reasonable delay
        assert!(result.delay >= 40.0 && result.delay <= 50.0);
        
        // Vector should have non-zero values
        let energy: i32 = result.vector.iter().map(|&x| x.0.abs() as i32).sum();
        assert!(energy > 0);
    }

    #[test]
    fn test_decode_vector() {
        let codebook = AdaptiveCodebook::new();
        
        // Decode at a specific delay
        let vector = codebook.decode_vector(50.0);
        
        // Should return a vector of correct size
        assert_eq!(vector.len(), SUBFRAME_SIZE);
    }
} 