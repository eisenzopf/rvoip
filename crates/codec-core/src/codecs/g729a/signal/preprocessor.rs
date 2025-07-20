//! Signal preprocessing with high-pass filtering

use crate::codecs::g729a::constants::*;
use crate::codecs::g729a::types::{AudioFrame, Q15};
use crate::codecs::g729a::math::fixed_point::FixedPointOps;

/// Signal preprocessor with high-pass filter
pub struct SignalPreprocessor {
    /// High-pass filter state [x(n-1), y(n-1)]
    hp_filter_state: [Q15; 2],
}

impl SignalPreprocessor {
    /// Create a new preprocessor
    pub fn new() -> Self {
        Self {
            hp_filter_state: [Q15::ZERO; 2],
        }
    }
    
    /// Process raw samples (convenience method for encoder)
    pub fn process(&mut self, input: &[i16]) -> Vec<Q15> {
        let frame = AudioFrame {
            samples: {
                let mut arr = [0i16; FRAME_SIZE];
                arr.copy_from_slice(&input[..FRAME_SIZE]);
                arr
            },
            timestamp: 0,
        };
        let processed = self.process_frame(&frame);
        processed.samples.iter().map(|&s| Q15(s)).collect()
    }
    
    /// Process a frame of audio samples
    /// Applies high-pass filtering to remove DC offset and low-frequency noise
    /// H(z) = (1 - z^-1) / (1 - 0.93*z^-1)
    pub fn process_frame(&mut self, input: &AudioFrame) -> AudioFrame {
        let mut output = AudioFrame {
            samples: [0i16; FRAME_SIZE],
            timestamp: input.timestamp,
        };
        
        // High-pass filter coefficients
        let b0 = Q15::ONE;
        let b1 = Q15::from_f32(-0.999); // -1.0 clamped to Q15 range
        let a1 = Q15::from_f32(-0.93);
        
        for i in 0..FRAME_SIZE {
            let x_n = Q15(input.samples[i]);
            
            // y[n] = x[n] - x[n-1] + 0.93*y[n-1]
            let b0_xn = b0.saturating_mul(x_n);
            let b1_xn1 = b1.saturating_mul(self.hp_filter_state[0]);
            let a1_yn1 = a1.saturating_mul(self.hp_filter_state[1]);
            
            let y_n = b0_xn.saturating_add(b1_xn1).saturating_add(Q15(-a1_yn1.0));
            
            // Update state
            self.hp_filter_state[0] = x_n;
            self.hp_filter_state[1] = y_n;
            
            // Store output
            output.samples[i] = y_n.0;
        }
        
        output
    }
    
    /// Reset the preprocessor state
    pub fn reset(&mut self) {
        self.hp_filter_state = [Q15::ZERO; 2];
    }
}

impl Default for SignalPreprocessor {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_preprocessor_creation() {
        let preprocessor = SignalPreprocessor::new();
        assert_eq!(preprocessor.hp_filter_state[0], Q15::ZERO);
        assert_eq!(preprocessor.hp_filter_state[1], Q15::ZERO);
    }

    #[test]
    fn test_dc_removal() {
        let mut preprocessor = SignalPreprocessor::new();
        
        // Create input with DC offset
        let mut input_samples = [1000i16; FRAME_SIZE];
        let input = AudioFrame {
            samples: input_samples,
            timestamp: 0,
        };
        
        // Process frame
        let output = preprocessor.process_frame(&input);
        
        // First sample should have high output due to step change
        assert!(output.samples[0].abs() > 500);
        
        // Later samples should decay toward zero (DC removed)
        let avg_last_10: i32 = output.samples[FRAME_SIZE-10..]
            .iter()
            .map(|&x| x as i32)
            .sum::<i32>() / 10;
        assert!(avg_last_10.abs() < 100);
    }

    #[test]
    fn test_passthrough_high_freq() {
        let mut preprocessor = SignalPreprocessor::new();
        
        // Create alternating signal (high frequency)
        let mut input_samples = [0i16; FRAME_SIZE];
        for i in 0..FRAME_SIZE {
            input_samples[i] = if i % 2 == 0 { 1000 } else { -1000 };
        }
        
        let input = AudioFrame {
            samples: input_samples,
            timestamp: 0,
        };
        
        // Process frame
        let output = preprocessor.process_frame(&input);
        
        // High frequency should pass through with minimal attenuation
        // Check that output maintains alternating pattern
        for i in 10..20 {
            if i % 2 == 0 {
                assert!(output.samples[i] > 500);
            } else {
                assert!(output.samples[i] < -500);
            }
        }
    }

    #[test]
    fn test_state_persistence() {
        let mut preprocessor = SignalPreprocessor::new();
        
        // Process first frame
        let input1 = AudioFrame {
            samples: [500i16; FRAME_SIZE],
            timestamp: 0,
        };
        let output1 = preprocessor.process_frame(&input1);
        
        // State should be updated
        assert_ne!(preprocessor.hp_filter_state[0], Q15::ZERO);
        assert_ne!(preprocessor.hp_filter_state[1], Q15::ZERO);
        
        // Process second frame with zeros
        let input2 = AudioFrame {
            samples: [0i16; FRAME_SIZE],
            timestamp: FRAME_SIZE as u64,
        };
        let output2 = preprocessor.process_frame(&input2);
        
        // First sample of second frame should show effect of previous state
        assert_ne!(output2.samples[0], 0);
    }

    #[test]
    fn test_reset() {
        let mut preprocessor = SignalPreprocessor::new();
        
        // Process a frame to change state
        let input = AudioFrame {
            samples: [1000i16; FRAME_SIZE],
            timestamp: 0,
        };
        preprocessor.process_frame(&input);
        
        // Verify state changed
        assert_ne!(preprocessor.hp_filter_state[0], Q15::ZERO);
        
        // Reset
        preprocessor.reset();
        
        // Verify state cleared
        assert_eq!(preprocessor.hp_filter_state[0], Q15::ZERO);
        assert_eq!(preprocessor.hp_filter_state[1], Q15::ZERO);
    }

    #[test]
    fn test_impulse_response() {
        let mut preprocessor = SignalPreprocessor::new();
        
        // Create impulse input
        let mut input_samples = [0i16; FRAME_SIZE];
        input_samples[0] = 10000;
        
        let input = AudioFrame {
            samples: input_samples,
            timestamp: 0,
        };
        
        // Process frame
        let output = preprocessor.process_frame(&input);
        
        // First output should be close to input (b0 = 1)
        assert!(output.samples[0].abs() > 8000);
        
        // Output should decay exponentially
        for i in 1..10 {
            assert!(output.samples[i].abs() < output.samples[i-1].abs());
        }
    }
} 