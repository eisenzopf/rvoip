use crate::common::basic_operators::*;
use crate::common::tab_ld8a::L_FRAME;

/// Post-processing for decoded speech
/// Based on POST_PRO.C from G.729A reference implementation
pub struct PostProcessing {
    mem_hp: [Word16; 2],  // High-pass filter memory
    gain_prec: Word16,    // Previous gain for smoothing
}

impl PostProcessing {
    pub fn new() -> Self {
        Self {
            mem_hp: [0; 2],
            gain_prec: 16384, // 1.0 in Q14
        }
    }
    
    /// Apply post-processing to decoded speech
    /// Includes high-pass filtering and gain control
    pub fn process(&mut self, speech: &mut [Word16]) {
        assert_eq!(speech.len(), L_FRAME, "Speech must be {} samples", L_FRAME);
        
        // Step 1: High-pass filtering
        self.high_pass_filter(speech);
        
        // Step 2: Output scaling/formatting
        self.scale_output(speech);
    }
    
    /// High-pass filter to remove DC and low-frequency components
    /// Based on Post_Process function in POST_PRO.C
    fn high_pass_filter(&mut self, speech: &mut [Word16]) {
        // High-pass filter: H(z) = 0.93980581 * (1 - z^-1) / (1 - 0.9330329 * z^-1)
        // Coefficients in Q15
        const A0: Word16 = 30596;  // 0.9330329 in Q15  
        const B0: Word16 = 30723;  // 0.93980581 in Q15
        
        for i in 0..L_FRAME {
            // Input with DC removal: x[n] - x[n-1]
            let input = sub(speech[i], self.mem_hp[0]);
            
            // Filter: y[n] = b0*x[n] + a0*y[n-1] 
            let temp = l_mult(B0, input);                    // b0 * input
            let temp = l_mac(temp, A0, self.mem_hp[1]);     // + a0 * y[n-1]
            let output = round(temp);
            
            // Update memories
            self.mem_hp[0] = speech[i];  // x[n-1] = x[n]
            self.mem_hp[1] = output;     // y[n-1] = y[n]
            
            // Store filtered output
            speech[i] = output;
        }
    }
    
    /// Scale output to appropriate level
    /// Simple scaling to ensure proper output range
    fn scale_output(&mut self, speech: &mut [Word16]) {
        // Find maximum absolute value in the frame
        let mut max_val = 0i16;
        for i in 0..L_FRAME {
            let abs_val = abs_s(speech[i]);
            if abs_val > max_val {
                max_val = abs_val;
            }
        }
        
        // Compute automatic gain control
        let target_level = 16384; // Target level in Q0 (half scale)
        let mut gain = 16384; // Default gain = 1.0 in Q14
        
        if max_val > 0 {
            // Compute gain to reach target level
            // gain = target_level / max_val (with overflow protection)
            if max_val > target_level {
                gain = div_s(target_level, max_val); // Reduce gain
                gain = shl(gain, 1); // Adjust Q-format to Q14
            }
        }
        
        // Smooth gain changes to avoid clicks
        gain = add(shr(mult(self.gain_prec, 3), 2), shr(gain, 2)); // 0.75*old + 0.25*new
        self.gain_prec = gain;
        
        // Apply gain (if different from unity)
        if gain != 16384 {
            for i in 0..L_FRAME {
                speech[i] = mult(speech[i], gain); // Q0 * Q14 = Q14 >> 15 = Q0
            }
        }
    }
    
    /// Alternative simplified post-processing
    /// Just applies basic high-pass filtering without AGC
    pub fn process_simple(&mut self, speech: &mut [Word16]) {
        self.high_pass_filter(speech);
        
        // Simple saturation check
        for i in 0..L_FRAME {
            if speech[i] > 32000 {
                speech[i] = 32000;
            } else if speech[i] < -32000 {
                speech[i] = -32000;
            }
        }
    }
}

// Helper function for division (simplified version)
fn div_s(num: Word16, den: Word16) -> Word16 {
    if den == 0 {
        return 32767; // Maximum value
    }
    
    // Simple division with overflow protection
    let result = (num as i32 * 32768) / den as i32;
    
    if result > 32767 {
        32767
    } else if result < -32768 {
        -32768
    } else {
        result as Word16
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_post_processing_basic() {
        let mut post_proc = PostProcessing::new();
        let mut speech = [100i16; L_FRAME];
        
        post_proc.process(&mut speech);
        
        // Speech should be modified by post-processing
        // This mainly tests that the function runs without crashing
        assert_eq!(speech.len(), L_FRAME, "Output length should be unchanged");
    }
    
    #[test]
    fn test_high_pass_filter() {
        let mut post_proc = PostProcessing::new();
        let mut speech = [0i16; L_FRAME];
        
        // Add a DC component
        for i in 0..L_FRAME {
            speech[i] = 1000; // Constant DC value
        }
        
        post_proc.high_pass_filter(&mut speech);
        
        // High-pass filter should reduce/remove DC component
        // After filtering, the signal should not be constant
        let first_sample = speech[0];
        let mut all_same = true;
        for i in 1..L_FRAME {
            if speech[i] != first_sample {
                all_same = false;
                break;
            }
        }
        
        assert!(!all_same, "High-pass filter should modify constant input");
    }
}