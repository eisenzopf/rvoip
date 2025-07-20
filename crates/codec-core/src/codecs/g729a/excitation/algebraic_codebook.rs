//! Algebraic codebook (fixed codebook) for G.729A

use crate::codecs::g729a::constants::*;
use crate::codecs::g729a::types::{Q15, Q31};
use crate::codecs::g729a::math::{FixedPointOps, dot_product};
use crate::codecs::g729a::signal::{backward_correlation, compute_phi_matrix};

/// Pulse position structure for G.729A
#[derive(Debug, Clone, Copy)]
pub struct PulsePosition {
    pub track: u8,      // Track number (0-3)
    pub position: u8,   // Position within subframe
    pub sign: bool,     // Pulse sign (true = positive)
}

/// Algebraic codebook contribution
#[derive(Debug, Clone)]
pub struct AlgebraicContribution {
    pub codebook_index: u32,
    pub pulses: [PulsePosition; 4],
    pub vector: [Q15; SUBFRAME_SIZE],
}

/// Algebraic codebook search structure
pub struct AlgebraicCodebook {
    /// Track pulse positions for 17-bit algebraic structure
    track_positions: [[u8; 10]; 4],
}

impl AlgebraicCodebook {
    /// Create a new algebraic codebook
    pub fn new() -> Self {
        // G.729A 17-bit algebraic structure
        // Track 0: m0 = 0, 5, 10, 15, 20, 25, 30, 35
        // Track 1: m1 = 1, 6, 11, 16, 21, 26, 31, 36
        // Track 2: m2 = 2, 7, 12, 17, 22, 27, 32, 37
        // Track 3: m3 = 3, 8, 13, 18, 23, 28, 33, 38 (+ special 4, 9, ..., 39)
        Self {
            track_positions: [
                [0, 5, 10, 15, 20, 25, 30, 35, 40, 40], // Track 0 (last two unused)
                [1, 6, 11, 16, 21, 26, 31, 36, 40, 40], // Track 1
                [2, 7, 12, 17, 22, 27, 32, 37, 40, 40], // Track 2
                [3, 8, 13, 18, 23, 28, 33, 38, 4, 39],  // Track 3 (includes 4, 39)
            ],
        }
    }
    
    /// Search for best algebraic codebook contribution
    pub fn search(&self, target: &[Q15], residual: &[Q15], h: &[Q15]) -> AlgebraicContribution {
        // 1. Backward filter target through h[n]
        let backward_filtered = backward_correlation(target, h);
        
        // 2. Compute autocorrelation of h[n] (phi matrix)
        let phi = compute_phi_matrix(h);
        
        // 3. For each track, find best pulse position
        let mut pulses = [PulsePosition::default(); 4];
        
        for track in 0..4 {
            let (position, sign) = self.search_track(
                track,
                &backward_filtered,
                &phi,
                &pulses,
            );
            
            pulses[track] = PulsePosition {
                track: track as u8,
                position,
                sign,
            };
        }
        
        // 4. Build codebook vector
        let vector = self.build_vector(&pulses);
        
        // 5. Compute codebook index
        let index = self.encode_pulses(&pulses);
        
        AlgebraicContribution {
            codebook_index: index,
            pulses,
            vector,
        }
    }
    
    /// Search for best pulse position in a track
    fn search_track(
        &self,
        track: usize,
        backward_filtered: &[Q15],
        phi: &[[Q31; SUBFRAME_SIZE]; SUBFRAME_SIZE],
        existing_pulses: &[PulsePosition; 4],
    ) -> (u8, bool) {
        let mut best_position = 0u8;
        let mut best_sign = true;
        let mut best_criterion = Q31(i32::MIN);
        
        // Number of positions to search (10 for track 3, 8 for others)
        let num_positions = if track == 3 { 10 } else { 8 };
        
        for i in 0..num_positions {
            let pos = self.track_positions[track][i];
            if pos >= SUBFRAME_SIZE as u8 {
                continue;
            }
            
            // Compute criterion for positive pulse
            let criterion_pos = self.compute_pulse_criterion(
                pos,
                true,
                track,
                backward_filtered,
                phi,
                existing_pulses,
            );
            
            // Compute criterion for negative pulse
            let criterion_neg = self.compute_pulse_criterion(
                pos,
                false,
                track,
                backward_filtered,
                phi,
                existing_pulses,
            );
            
            // Select best
            if criterion_pos.0 > best_criterion.0 {
                best_criterion = criterion_pos;
                best_position = pos;
                best_sign = true;
            }
            
            if criterion_neg.0 > best_criterion.0 {
                best_criterion = criterion_neg;
                best_position = pos;
                best_sign = false;
            }
        }
        
        (best_position, best_sign)
    }
    
    /// Compute selection criterion for a pulse
    fn compute_pulse_criterion(
        &self,
        position: u8,
        sign: bool,
        track: usize,
        backward_filtered: &[Q15],
        phi: &[[Q31; SUBFRAME_SIZE]; SUBFRAME_SIZE],
        existing_pulses: &[PulsePosition; 4],
    ) -> Q31 {
        let pos = position as usize;
        let pulse_value = if sign { Q15::ONE } else { Q15(Q15_ONE.saturating_neg()) };
        
        // Numerator: correlation with target
        let mut numerator = backward_filtered[pos].to_q31();
        if !sign {
            numerator = Q31(numerator.0.saturating_neg());
        }
        
        // Consider correlation with existing pulses
        for i in 0..track {
            if existing_pulses[i].position < SUBFRAME_SIZE as u8 {
                let other_pos = existing_pulses[i].position as usize;
                let other_sign = if existing_pulses[i].sign { 1 } else { -1 };
                
                let cross_term = phi[pos.min(other_pos)][pos.max(other_pos)];
                let contribution = Q31(cross_term.0 * other_sign);
                
                numerator = numerator.saturating_add(contribution);
            }
        }
        
        // Denominator: energy term
        let energy = phi[pos][pos];
        
        // Approximate criterion: num^2 / den
        if energy.0 > 0 {
            // Compute num^2
            let num_sq = (numerator.0 as i64 * numerator.0 as i64 >> 31) as i32;
            Q31(num_sq / (energy.0 >> 16).max(1))
        } else {
            Q31::ZERO
        }
    }
    
    /// Build codebook vector from pulses
    pub fn build_vector(&self, pulses: &[PulsePosition]) -> [Q15; SUBFRAME_SIZE] {
        let mut vector = [Q15::ZERO; SUBFRAME_SIZE];
        
        for pulse in pulses {
            if pulse.position < SUBFRAME_SIZE as u8 {
                let value = if pulse.sign { Q15_ONE } else { -Q15_ONE };
                vector[pulse.position as usize] = Q15(value);
            }
        }
        
        vector
    }
    
    /// Encode pulses to 17-bit index
    fn encode_pulses(&self, pulses: &[PulsePosition]) -> u32 {
        let mut index = 0u32;
        
        // Track 0: 3 bits for position
        for i in 0..8 {
            if self.track_positions[0][i] == pulses[0].position {
                index |= i as u32;
                break;
            }
        }
        
        // Track 1: 3 bits for position
        for i in 0..8 {
            if self.track_positions[1][i] == pulses[1].position {
                index |= (i as u32) << 3;
                break;
            }
        }
        
        // Track 2: 3 bits for position
        for i in 0..8 {
            if self.track_positions[2][i] == pulses[2].position {
                index |= (i as u32) << 6;
                break;
            }
        }
        
        // Track 3: 4 bits for position (10 positions)
        for i in 0..10 {
            if self.track_positions[3][i] == pulses[3].position {
                index |= (i as u32) << 9;
                break;
            }
        }
        
        // Signs: 4 bits
        if pulses[0].sign { index |= 1 << 13; }
        if pulses[1].sign { index |= 1 << 14; }
        if pulses[2].sign { index |= 1 << 15; }
        if pulses[3].sign { index |= 1 << 16; }
        
        index
    }
    
    /// Decode 17-bit index to pulses
    pub fn decode_pulses(&self, index: u32) -> [PulsePosition; 4] {
        let mut pulses = [PulsePosition::default(); 4];
        
        // Track 0: bits 0-2
        let pos0_idx = (index & 0x7) as usize;
        pulses[0] = PulsePosition {
            track: 0,
            position: self.track_positions[0][pos0_idx],
            sign: (index >> 13) & 1 != 0,
        };
        
        // Track 1: bits 3-5
        let pos1_idx = ((index >> 3) & 0x7) as usize;
        pulses[1] = PulsePosition {
            track: 1,
            position: self.track_positions[1][pos1_idx],
            sign: (index >> 14) & 1 != 0,
        };
        
        // Track 2: bits 6-8
        let pos2_idx = ((index >> 6) & 0x7) as usize;
        pulses[2] = PulsePosition {
            track: 2,
            position: self.track_positions[2][pos2_idx],
            sign: (index >> 15) & 1 != 0,
        };
        
        // Track 3: bits 9-12
        let pos3_idx = ((index >> 9) & 0xF) as usize;
        pulses[3] = PulsePosition {
            track: 3,
            position: self.track_positions[3][pos3_idx.min(9)],
            sign: (index >> 16) & 1 != 0,
        };
        
        pulses
    }
}

impl Default for AlgebraicCodebook {
    fn default() -> Self {
        Self::new()
    }
}

impl Default for PulsePosition {
    fn default() -> Self {
        Self {
            track: 0,
            position: 0,
            sign: true,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_algebraic_codebook_creation() {
        let codebook = AlgebraicCodebook::new();
        
        // Check track positions
        assert_eq!(codebook.track_positions[0][0], 0);
        assert_eq!(codebook.track_positions[1][0], 1);
        assert_eq!(codebook.track_positions[2][0], 2);
        assert_eq!(codebook.track_positions[3][0], 3);
        
        // Check special positions for track 3
        assert_eq!(codebook.track_positions[3][8], 4);
        assert_eq!(codebook.track_positions[3][9], 39);
    }

    #[test]
    fn test_build_vector() {
        let codebook = AlgebraicCodebook::new();
        
        let pulses = [
            PulsePosition { track: 0, position: 5, sign: true },
            PulsePosition { track: 1, position: 11, sign: false },
            PulsePosition { track: 2, position: 17, sign: true },
            PulsePosition { track: 3, position: 23, sign: false },
        ];
        
        let vector = codebook.build_vector(&pulses);
        
        // Check pulses are at correct positions
        assert_eq!(vector[5], Q15(Q15_ONE));
        assert_eq!(vector[11], Q15(Q15_ONE.saturating_neg()));
        assert_eq!(vector[17], Q15::ONE);
        assert_eq!(vector[23], Q15(Q15_ONE.saturating_neg()));
        
        // Check other positions are zero
        assert_eq!(vector[0], Q15::ZERO);
        assert_eq!(vector[10], Q15::ZERO);
    }

    #[test]
    fn test_encode_decode_pulses() {
        let codebook = AlgebraicCodebook::new();
        
        let original_pulses = [
            PulsePosition { track: 0, position: 10, sign: true },  // idx 2
            PulsePosition { track: 1, position: 16, sign: false }, // idx 3
            PulsePosition { track: 2, position: 27, sign: true },  // idx 5
            PulsePosition { track: 3, position: 38, sign: false }, // idx 7
        ];
        
        // Encode
        let index = codebook.encode_pulses(&original_pulses);
        
        // Decode
        let decoded_pulses = codebook.decode_pulses(index);
        
        // Check they match
        for i in 0..4 {
            assert_eq!(decoded_pulses[i].track, original_pulses[i].track);
            assert_eq!(decoded_pulses[i].position, original_pulses[i].position);
            assert_eq!(decoded_pulses[i].sign, original_pulses[i].sign);
        }
    }

    #[test]
    fn test_search_simple() {
        let codebook = AlgebraicCodebook::new();
        
        // Create simple target and residual
        let mut target = vec![Q15::ZERO; SUBFRAME_SIZE];
        let mut h = vec![Q15::ZERO; SUBFRAME_SIZE];
        
        // Put energy at specific positions
        target[5] = Q15::from_f32(0.5);
        target[11] = Q15::from_f32(-0.3);
        
        // Simple impulse response
        h[0] = Q15::ONE;
        
        let residual = vec![Q15::ZERO; SUBFRAME_SIZE];
        
        let result = codebook.search(&target, &residual, &h);
        
        // Should have 4 pulses
        assert_eq!(result.pulses.len(), 4);
        
        // Index should be encodable
        assert!(result.codebook_index < (1 << 17));
        
        // Vector should have pulses
        let energy: i32 = result.vector.iter().map(|&x| x.0.abs() as i32).sum();
        assert!(energy > 0);
    }
} 