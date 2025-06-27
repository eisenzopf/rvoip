//! Audio Resampler - Sample rate conversion
//!
//! This module implements sample rate conversion using linear interpolation
//! for basic resampling needs.

use tracing::{debug, warn};
use crate::error::{Result, AudioProcessingError};

/// Configuration for resampler
#[derive(Debug, Clone)]
pub struct ResamplerConfig {
    /// Input sample rate
    pub input_rate: u32,
    /// Output sample rate  
    pub output_rate: u32,
    /// Quality level (0-10, higher = better quality)
    pub quality: u8,
}

/// Audio resampler for sample rate conversion
pub struct Resampler {
    /// Resampler configuration
    config: ResamplerConfig,
    /// Conversion ratio (output_rate / input_rate)
    ratio: f64,
    /// Current position in the input stream (fractional)
    position: f64,
    /// Previous sample for interpolation
    prev_sample: i16,
    /// Whether this is the first sample
    first_sample: bool,
}

impl Resampler {
    /// Create a new resampler
    pub fn new(input_rate: u32, output_rate: u32, quality: u8) -> Result<Self> {
        if input_rate == 0 || output_rate == 0 {
            return Err(AudioProcessingError::ResamplingFailed {
                from_rate: input_rate,
                to_rate: output_rate,
            }.into());
        }
        
        if quality > 10 {
            warn!("Resampler quality {} clamped to 10", quality);
        }
        
        let ratio = output_rate as f64 / input_rate as f64;
        
        debug!("Creating resampler: {}Hz -> {}Hz (ratio: {:.4})", 
               input_rate, output_rate, ratio);
        
        Ok(Self {
            config: ResamplerConfig {
                input_rate,
                output_rate,
                quality: quality.min(10),
            },
            ratio,
            position: 0.0,
            prev_sample: 0,
            first_sample: true,
        })
    }
    
    /// Resample audio samples
    pub fn resample(&mut self, input_samples: &[i16]) -> Result<Vec<i16>> {
        if input_samples.is_empty() {
            return Ok(Vec::new());
        }
        
        // Calculate expected output length
        let expected_output_len = ((input_samples.len() as f64) * self.ratio).ceil() as usize;
        let mut output_samples = Vec::with_capacity(expected_output_len);
        
        // Reset position for each new frame (simple approach)
        self.position = 0.0;
        
        // Generate output samples
        while self.position < input_samples.len() as f64 {
            let sample = self.interpolate_sample(input_samples)?;
            output_samples.push(sample);
            
            // Advance position
            self.position += 1.0 / self.ratio;
        }
        
        // Update state for next frame
        if !input_samples.is_empty() {
            self.prev_sample = input_samples[input_samples.len() - 1];
            self.first_sample = false;
        }
        
        Ok(output_samples)
    }
    
    /// Reset resampler state
    pub fn reset(&mut self) {
        self.position = 0.0;
        self.prev_sample = 0;
        self.first_sample = true;
        debug!("Resampler reset");
    }
    
    /// Get conversion ratio
    pub fn ratio(&self) -> f64 {
        self.ratio
    }
    
    /// Get configuration
    pub fn config(&self) -> &ResamplerConfig {
        &self.config
    }
    
    /// Interpolate sample at current position
    fn interpolate_sample(&self, input_samples: &[i16]) -> Result<i16> {
        let index = self.position as usize;
        let fraction = self.position - index as f64;
        
        // Handle edge cases
        if index >= input_samples.len() {
            return Ok(self.prev_sample);
        }
        
        let current_sample = input_samples[index];
        
        // If no interpolation needed (exact sample)
        if fraction == 0.0 {
            return Ok(current_sample);
        }
        
        // Get next sample for interpolation
        let next_sample = if index + 1 < input_samples.len() {
            input_samples[index + 1]
        } else {
            // Use previous sample if at end
            current_sample
        };
        
        // Linear interpolation based on quality setting
        let interpolated = match self.config.quality {
            0..=2 => {
                // Nearest neighbor (no interpolation)
                if fraction < 0.5 { current_sample } else { next_sample }
            }
            3..=6 => {
                // Linear interpolation
                self.linear_interpolate(current_sample, next_sample, fraction)
            }
            7..=10 => {
                // Enhanced linear interpolation with smoothing
                self.smooth_interpolate(input_samples, index, fraction)
            }
            _ => current_sample, // Should not happen due to clamping
        };
        
        Ok(interpolated)
    }
    
    /// Perform linear interpolation between two samples
    fn linear_interpolate(&self, sample1: i16, sample2: i16, fraction: f64) -> i16 {
        let result = sample1 as f64 + (sample2 as f64 - sample1 as f64) * fraction;
        result.round() as i16
    }
    
    /// Perform smooth interpolation with neighboring samples
    fn smooth_interpolate(&self, input_samples: &[i16], index: usize, fraction: f64) -> i16 {
        // Use more samples for smoother interpolation
        let prev_sample = if index > 0 {
            input_samples[index - 1]
        } else if !self.first_sample {
            self.prev_sample
        } else {
            input_samples[index]
        };
        
        let current_sample = input_samples[index];
        let next_sample = if index + 1 < input_samples.len() {
            input_samples[index + 1]
        } else {
            current_sample
        };
        
        let next_next_sample = if index + 2 < input_samples.len() {
            input_samples[index + 2]
        } else {
            next_sample
        };
        
        // Cubic interpolation (simplified)
        let t = fraction;
        let t2 = t * t;
        let t3 = t2 * t;
        
        // Catmull-Rom spline coefficients
        let a0 = -0.5 * prev_sample as f64 + 1.5 * current_sample as f64 
                 - 1.5 * next_sample as f64 + 0.5 * next_next_sample as f64;
        let a1 = prev_sample as f64 - 2.5 * current_sample as f64 
                + 2.0 * next_sample as f64 - 0.5 * next_next_sample as f64;
        let a2 = -0.5 * prev_sample as f64 + 0.5 * next_sample as f64;
        let a3 = current_sample as f64;
        
        let result = a0 * t3 + a1 * t2 + a2 * t + a3;
        
        // Clamp to 16-bit range
        result.max(i16::MIN as f64).min(i16::MAX as f64).round() as i16
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_resampler_creation() {
        let resampler = Resampler::new(8000, 16000, 5);
        assert!(resampler.is_ok());
        
        let resampler = resampler.unwrap();
        assert_eq!(resampler.ratio(), 2.0);
    }
    
    #[test]
    fn test_upsampling() {
        let mut resampler = Resampler::new(8000, 16000, 5).unwrap();
        let input = vec![100, 200, 300, 400];
        let output = resampler.resample(&input).unwrap();
        
        // Should approximately double the number of samples
        assert!(output.len() >= input.len() * 2 - 1);
        assert!(output.len() <= input.len() * 2 + 1);
    }
    
    #[test]
    fn test_downsampling() {
        let mut resampler = Resampler::new(16000, 8000, 5).unwrap();
        let input = vec![100, 150, 200, 250, 300, 350, 400, 450];
        let output = resampler.resample(&input).unwrap();
        
        // Should approximately halve the number of samples
        assert!(output.len() >= input.len() / 2 - 1);
        assert!(output.len() <= input.len() / 2 + 1);
    }
} 