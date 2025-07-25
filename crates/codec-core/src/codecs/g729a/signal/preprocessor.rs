//! Signal preprocessing with pre-emphasis and high-pass filtering

use crate::codecs::g729a::types::{AudioFrame, Q15};

/// ITU-T G.729A signal preprocessor with exact pre-emphasis and high-pass filtering
pub struct Preprocessor {
    // Pre-emphasis filter state: H(z) = 1 - 0.68z^-1
    preemph_mem: i16,  // x[n-1] for pre-emphasis
    
    // High-pass filter state (140Hz cutoff)
    // Filter state - previous outputs (y[n-1], y[n-2])
    output_y1: i32,  // Q15.12 format
    output_y2: i32,  // Q15.12 format
    // Filter state - previous inputs (x[n-1], x[n-2])
    input_x0: i16,   // Q15.0 format
    input_x1: i16,   // Q15.0 format
}

impl Preprocessor {
    /// Create a new preprocessor
    pub fn new() -> Self {
        Self {
            preemph_mem: 0,
            output_y1: 0,
            output_y2: 0,
            input_x0: 0,
            input_x1: 0,
        }
    }

    /// Process a frame through the ITU-T G.729A preprocessing pipeline
    pub fn process(&mut self, samples: &[i16]) -> Vec<Q15> {
        // Step 1: Apply pre-emphasis filter H(z) = 1 - 0.68z^-1
        let preemphasized = self.apply_preemphasis(samples);
        
        // Step 2: Apply high-pass filter (140Hz cutoff)
        let highpass_filtered = self.apply_highpass_filter(&preemphasized);
        
        highpass_filtered
    }
    
    /// Apply ITU-T G.729A pre-emphasis filter: H(z) = 1 - 0.68z^-1
    fn apply_preemphasis(&mut self, samples: &[i16]) -> Vec<i16> {
        // Pre-emphasis coefficient: 0.68 in Q15 format
        const PREEMPH_FACTOR: i16 = 22282; // 0.68 * 32768
        
        let mut result = Vec::with_capacity(samples.len());
        
        for &sample in samples {
            // y[n] = x[n] - 0.68 * x[n-1]
            let preemph_contrib = ((self.preemph_mem as i32 * PREEMPH_FACTOR as i32) + 16384) >> 15;
            let output = sample as i32 - preemph_contrib;
            
            // Saturate to 16-bit range
            let saturated = output.clamp(-32768, 32767) as i16;
            result.push(saturated);
            
            // Update memory
            self.preemph_mem = sample;
        }
        
        result
    }
    
    /// Apply high-pass filter (140Hz cutoff) 
    fn apply_highpass_filter(&mut self, samples: &[i16]) -> Vec<Q15> {
        // Filter coefficients (140Hz cutoff)
        // Coefficients are stored in Q1.12 for A1 and Q0.12 for others
        const A1: i16 = 7807;   // Q1.12
        const A2: i16 = -3733;  // Q0.12  
        const B0: i16 = 1899;   // Q0.12
        const B1: i16 = -3798;  // Q0.12
        const B2: i16 = 1899;   // Q0.12
        
        const MAXINT28: i32 = 0x07FFFFFF;
        
        let mut result = Vec::with_capacity(samples.len());
        
        for &sample in samples {
            let input_x2 = self.input_x1;
            self.input_x1 = self.input_x0;
            self.input_x0 = sample;
            
            // y[i] = B0*x[i] + B1*x[i-1] + B2*x[i-2] + A1*y[i-1] + A2*y[i-2]
            
            // Start with feedback terms
            let mut acc: i32 = mult16_32_q12(A1, self.output_y1);
            acc = mac16_32_q12(acc, A2, self.output_y2);
            
            // Add feedforward terms  
            acc = mac16_16(acc, self.input_x0, B0);
            acc = mac16_16(acc, self.input_x1, B1);
            acc = mac16_16(acc, input_x2, B2);
            
            // Saturate to prevent overflow
            acc = saturate(acc, MAXINT28);
            
            // Extract integer part (Q15.0) from Q15.12
            let output = pshr(acc, 12);
            result.push(Q15(output));
            
            // Update state
            self.output_y2 = self.output_y1;
            self.output_y1 = acc;
        }
        
        result
    }
}

// Fixed-point arithmetic helpers

/// Multiply 16-bit by 32-bit with Q12 scaling
fn mult16_32_q12(a: i16, b: i32) -> i32 {
    let a32 = a as i32;
    // Split b into high and low parts for precise calculation
    let b_high = b >> 12;
    let b_low = b & 0x00000fff;
    
    (a32 * b_high) + ((a32 * b_low) >> 12)
}

/// Multiply-accumulate 16-bit by 32-bit with Q12 scaling
fn mac16_32_q12(c: i32, a: i16, b: i32) -> i32 {
    c.saturating_add(mult16_32_q12(a, b))
}

/// Multiply-accumulate 16-bit by 16-bit
fn mac16_16(c: i32, a: i16, b: i16) -> i32 {
    let product = (a as i32) * (b as i32);
    c.saturating_add(product)
}

/// Saturate to given maximum value
fn saturate(x: i32, max_val: i32) -> i32 {
    if x > max_val {
        max_val
    } else if x < -(max_val + 1) {
        -(max_val + 1)
    } else {
        x
    }
}

/// Shift right with rounding
fn pshr(a: i32, shift: u32) -> i16 {
    let rounded = a + (1 << (shift - 1));
    (rounded >> shift) as i16
} 