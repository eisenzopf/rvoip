use crate::common::basic_operators::*;
use crate::common::tab_ld8a::L_SUBFR;

/// ACELP (Algebraic Codebook) decoder
/// Based on DE_ACELP.C from G.729A reference implementation
pub struct AcelpDecoder {
    // No persistent state needed for ACELP decoding
}

impl AcelpDecoder {
    pub fn new() -> Self {
        Self {}
    }
    
    /// Decode ACELP fixed codebook vector from position and sign indices
    /// Based on Decod_ACELP function in DE_ACELP.C
    pub fn decode_acelp(
        &self,
        position_index: Word16,  // 13-bit position index
        sign_index: Word16,      // 4-bit sign index  
        fixed_vector: &mut [Word16] // Output: decoded fixed vector Q13
    ) {
        assert_eq!(fixed_vector.len(), L_SUBFR, "Fixed vector must be length {}", L_SUBFR);
        
        // Clear the output vector
        for i in 0..L_SUBFR {
            fixed_vector[i] = 0;
        }
        
        // G.729A uses 4 pulses in each 40-sample subframe
        // Extract pulse positions from the 13-bit index
        let mut positions = [0i16; 4];
        let mut temp_index = position_index;
        
        // Decode positions using combinatorial decoding
        // Track structure for G.729A ACELP:
        // Track 0: positions 0, 5, 10, 15, 20, 25, 30, 35
        // Track 1: positions 1, 6, 11, 16, 21, 26, 31, 36  
        // Track 2: positions 2, 7, 12, 17, 22, 27, 32, 37
        // Track 3: positions 3, 8, 13, 18, 23, 28, 33, 38
        
        positions[0] = mult(temp_index & 7, 5);         // Track 0
        temp_index = shr(temp_index, 3);
        
        positions[1] = add(mult(temp_index & 7, 5), 1); // Track 1
        temp_index = shr(temp_index, 3);
        
        positions[2] = add(mult(temp_index & 7, 5), 2); // Track 2
        temp_index = shr(temp_index, 3);
        
        positions[3] = add(mult(temp_index & 7, 5), 3); // Track 3
        
        // Ensure positions are within bounds
        for i in 0..4 {
            if positions[i] >= L_SUBFR as Word16 {
                positions[i] = sub(positions[i], 5);
            }
        }
        
        // Extract signs from the 4-bit sign index
        let mut signs = [1i16; 4];
        for i in 0..4 {
            if (sign_index & (1 << i)) != 0 {
                signs[i] = -1;
            }
        }
        
        // Place pulses in the fixed vector
        for i in 0..4 {
            let pos = positions[i] as usize;
            if pos < L_SUBFR {
                // G.729A uses unit amplitude pulses (±8192 in Q13)
                fixed_vector[pos] = mult(signs[i], 8192); // ±1.0 in Q13
            }
        }
    }
}