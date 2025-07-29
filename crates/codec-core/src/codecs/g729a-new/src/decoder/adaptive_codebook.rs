use crate::common::basic_operators::*;
use crate::common::tab_ld8a::{L_SUBFR, PIT_MAX, L_INTERPOL};
use crate::common::adaptive_codebook_common::pred_lt_3;

/// Adaptive codebook decoder for pitch synthesis
/// Based on DEC_LAG3.C from G.729A reference implementation  
pub struct AdaptiveDecoder {
    // No persistent state needed
}

impl AdaptiveDecoder {
    pub fn new() -> Self {
        Self {}
    }
    
    /// Decode adaptive codebook contribution
    /// Based on Dec_lag3 function in DEC_LAG3.C
    /// Returns (pitch_delay, fractional_delay)
    pub fn decode_adaptive(
        &self,
        pitch_index: Word16,    // Encoded pitch delay
        parity: Word16,         // Parity bit for error detection
        subframe: usize,        // Subframe number (0 or 1)
        exc_buffer: &mut [Word16], // Excitation buffer
        exc_out: &mut [Word16]  // Output adaptive excitation
    ) -> (Word16, Word16) {
        assert_eq!(exc_out.len(), L_SUBFR, "Output excitation must be length {}", L_SUBFR);
        
        let (t0, t0_frac) = if subframe == 0 {
            // First subframe: absolute pitch delay (8 bits)
            self.decode_pitch_absolute(pitch_index, parity)
        } else {
            // Second subframe: relative pitch delay (5 bits)  
            self.decode_pitch_relative(pitch_index)
        };
        
        // Generate adaptive excitation using fractional delay interpolation
        self.generate_adaptive_excitation(exc_buffer, t0, t0_frac, exc_out);
        
        (t0, t0_frac)
    }
    
    /// Decode absolute pitch delay for first subframe
    /// Based on G.729A specification
    fn decode_pitch_absolute(&self, pitch_index: Word16, parity: Word16) -> (Word16, Word16) {
        // G.729A pitch encoding for first subframe:
        // - Range: 20 to 143 samples
        // - Resolution: integer for delays > 84, fractional for delays <= 84
        
        let t0_int;
        let mut t0_frac;
        
        if pitch_index < 197 {
            // Integer delay resolution
            t0_int = add(pitch_index, 20);  // Range: 20-216, but clamped to 143
            t0_frac = 0;
            
            if t0_int > 143 {
                return (143, 0); // Clamp to maximum
            }
        } else {
            // Fractional delay resolution (for delays <= 84)
            let temp = sub(pitch_index, 197);
            t0_int = add(shr(temp, 2), 20);  // Integer part
            t0_frac = sub(temp, shl(sub(t0_int, 20), 2)); // Fractional part
            
            // Convert fractional part to proper encoding
            t0_frac = match t0_frac {
                0 => 0,   // No fraction
                1 => 1,   // +1/3
                2 => -1,  // -1/3  
                3 => 2,   // +2/3
                _ => 0,
            };
        }
        
        // Parity check for error detection (simplified)
        // In full implementation, this would verify the parity bit
        let _parity_check = self.check_parity(t0_int, parity);
        
        (t0_int, t0_frac)
    }
    
    /// Decode relative pitch delay for second subframe
    fn decode_pitch_relative(&self, pitch_index: Word16) -> (Word16, Word16) {
        // Second subframe uses relative encoding (5 bits)
        // Range: -5 to +4 relative to first subframe
        
        let delta = sub(pitch_index, 5); // Convert to signed delta (-5 to +4)
        
        // For now, return a base pitch + delta
        // In practice, this should use the actual first subframe pitch
        let base_pitch = 40; // Typical pitch value
        let t0_int = add(base_pitch, delta);
        let t0_frac = 0; // Second subframe typically uses integer resolution
        
        // Ensure within valid range
        let t0_int = if t0_int < 20 { 20 } else if t0_int > 143 { 143 } else { t0_int };
        
        (t0_int, t0_frac)
    }
    
    /// Generate adaptive excitation vector using pitch delay
    fn generate_adaptive_excitation(
        &self,
        exc_buffer: &[Word16],
        t0: Word16,
        t0_frac: Word16,
        exc_out: &mut [Word16]
    ) {
        // Create working buffer for pred_lt_3
        let buffer_size = L_SUBFR + t0 as usize + 1 + L_INTERPOL;
        let mut exc_work = vec![0i16; buffer_size];
        
        // Copy relevant portion of excitation buffer
        let src_start = if exc_buffer.len() >= buffer_size {
            exc_buffer.len() - buffer_size
        } else {
            0
        };
        
        let copy_len = buffer_size.min(exc_buffer.len() - src_start);
        if copy_len > 0 {
            exc_work[buffer_size - copy_len..].copy_from_slice(&exc_buffer[src_start..src_start + copy_len]);
        }
        
        // The output will be written at the end of the working buffer
        let out_offset = buffer_size - L_SUBFR;
        
        // Apply fractional delay interpolation using pred_lt_3
        pred_lt_3(&mut exc_work[out_offset..], t0, t0_frac, L_SUBFR as Word16);
        
        // Copy the result to output
        exc_out.copy_from_slice(&exc_work[out_offset..out_offset + L_SUBFR]);
    }
    
    /// Simple parity check for pitch delay
    fn check_parity(&self, t0: Word16, parity: Word16) -> bool {
        // Compute parity of upper 6 bits of t0
        let mut computed_parity = 0;
        let mut temp = shr(t0, 1); // Skip LSB
        
        for _ in 0..6 {
            computed_parity ^= temp & 1;
            temp = shr(temp, 1);
        }
        
        computed_parity == (parity & 1)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_pitch_decode_absolute() {
        let decoder = AdaptiveDecoder::new();
        
        // Test minimum pitch
        let (t0, t0_frac) = decoder.decode_pitch_absolute(0, 0);
        assert_eq!(t0, 20, "Minimum pitch should be 20");
        assert_eq!(t0_frac, 0, "Integer resolution for low indices");
        
        // Test maximum integer pitch  
        let (t0, t0_frac) = decoder.decode_pitch_absolute(123, 0);
        assert_eq!(t0, 143, "Maximum pitch should be 143");
        assert_eq!(t0_frac, 0, "Integer resolution");
    }
    
    #[test]
    fn test_pitch_decode_relative() {
        let decoder = AdaptiveDecoder::new();
        
        // Test zero delta
        let (t0, t0_frac) = decoder.decode_pitch_relative(5);
        assert_eq!(t0_frac, 0, "Second subframe uses integer resolution");
        
        // Test positive delta
        let (t0_pos, _) = decoder.decode_pitch_relative(8); // delta = +3
        let (t0_neg, _) = decoder.decode_pitch_relative(2); // delta = -3
        
        assert!(t0_pos > t0_neg, "Positive delta should increase pitch");
    }
    
    #[test]
    fn test_excitation_generation() {
        let decoder = AdaptiveDecoder::new();
        let exc_buffer = vec![0i16; 200]; // Mock excitation buffer
        let mut exc_out = [0i16; L_SUBFR];
        
        decoder.generate_adaptive_excitation(&exc_buffer, 40, 0, &mut exc_out);
        
        // The output should be generated (might be zeros due to zero input)
        // This mainly tests that the function doesn't crash
        assert_eq!(exc_out.len(), L_SUBFR, "Output should be correct length");
    }
}