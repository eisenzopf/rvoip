//! Bitstream packing and unpacking for G.729A
//! 
//! Implements bit allocation according to ITU-T Table 8/G.729:
//! - Total: 80 bits (10 bytes)
//! - L0: 7 bits (1st stage LSP)
//! - L1,L2,L3: 5 bits each (2nd stage LSP)
//! - P1: 8 bits, P0: 1 bit (1st subframe pitch)
//! - P2: 5 bits, P3: 2 bits (2nd subframe pitch)
//! - C1,C2: 17 bits each (fixed codebook)
//! - GA1,GA2: 3 bits each (stage 1 gains)
//! - GB1,GB2: 4 bits each (stage 2 gains)

use crate::codecs::g729a::types::{EncodedFrame, DecodedParameters};

/// Pack encoded parameters into 80-bit (10-byte) frame per ITU-T spec
pub fn pack_frame(params: &EncodedFrame) -> [u8; 10] {
    let mut packed = [0u8; 10];
    let mut bits = [0u8; 80]; // Work in bits first
    let mut bit_idx = 0;
    
    // Bit 1: MA predictor switch (1 bit) - set to 0 for standard mode
    pack_bits(&mut bits, &mut bit_idx, 0, 1);
    
    // Bits 2-8: L0 - 1st stage LSP VQ (7 bits)
    pack_bits(&mut bits, &mut bit_idx, params.lsp_indices[0] as u32, 7);
    
    // Bits 9-13: L1 - 2nd stage lower LSP VQ (5 bits)
    pack_bits(&mut bits, &mut bit_idx, params.lsp_indices[1] as u32, 5);
    
    // Bits 14-18: L2 - 2nd stage lower LSP VQ (5 bits)  
    pack_bits(&mut bits, &mut bit_idx, params.lsp_indices[2] as u32, 5);
    
    // First subframe
    // Bits 19-26: P1 - pitch period (8 bits)
    pack_bits(&mut bits, &mut bit_idx, params.pitch_delay_int[0] as u32, 8);
    
    // Bit 27: Parity check on pitch
    let parity = compute_pitch_parity(params.pitch_delay_int[0]);
    pack_bits(&mut bits, &mut bit_idx, parity as u32, 1);
    
    // Decode algebraic index into positions and signs
    let (signs1, positions1) = decode_algebraic_index(params.fixed_codebook_idx[0]);
    
    // Bits 28-40: CB1 - codebook pulse positions (13 bits)
    pack_bits(&mut bits, &mut bit_idx, positions1, 13);
    
    // Bits 41-44: S1 - codebook pulse signs (4 bits)
    pack_bits(&mut bits, &mut bit_idx, signs1, 4);
    
    // Bits 45-47: GA1 - pitch and codebook gains stage 1 (3 bits)
    pack_bits(&mut bits, &mut bit_idx, params.gain_indices[0][0] as u32, 3);
    
    // Bits 48-51: GB1 - pitch and codebook gains stage 2 (4 bits)
    pack_bits(&mut bits, &mut bit_idx, params.gain_indices[0][1] as u32, 4);
    
    // Second subframe
    // Bits 52-56: P2 - pitch period relative (5 bits)
    pack_bits(&mut bits, &mut bit_idx, params.pitch_delay_int[1] as u32, 5);
    
    // Decode algebraic index for second subframe
    let (signs2, positions2) = decode_algebraic_index(params.fixed_codebook_idx[1]);
    
    // Bits 57-69: CB2 - codebook pulse positions (13 bits)
    pack_bits(&mut bits, &mut bit_idx, positions2, 13);
    
    // Bits 70-73: S2 - codebook pulse signs (4 bits)
    pack_bits(&mut bits, &mut bit_idx, signs2, 4);
    
    // Bits 74-76: GA2 - pitch and codebook gains stage 1 (3 bits)
    pack_bits(&mut bits, &mut bit_idx, params.gain_indices[1][0] as u32, 3);
    
    // Bits 77-80: GB2 - pitch and codebook gains stage 2 (4 bits)
    pack_bits(&mut bits, &mut bit_idx, params.gain_indices[1][1] as u32, 4);
    
    // Note: L3 (2nd stage upper LSP VQ) is NOT in the bitstream!
    // Total: 1+7+5+5+8+1+13+4+3+4+5+13+4+3+4 = 80 bits exactly
    
    // Convert bits to bytes
    for i in 0..10 {
        for j in 0..8 {
            packed[i] = (packed[i] << 1) | bits[i * 8 + j];
        }
    }
    
    packed
}

/// Unpack 80-bit frame into decoded parameters per ITU-T spec
pub fn unpack_frame(packed: &[u8; 10]) -> DecodedParameters {
    let mut bits = [0u8; 80];
    let mut bit_idx = 0;
    
    // Convert bytes to bits
    for i in 0..10 {
        for j in 0..8 {
            bits[i * 8 + j] = (packed[i] >> (7 - j)) & 1;
        }
    }
    
    // Bit 1: MA predictor switch (1 bit)
    let _ma_mode = bits[bit_idx];
    bit_idx += 1;
    
    // Bits 2-8: L0 (7 bits)
    let l0 = unpack_bits(&bits, &mut bit_idx, 7) as u8;
    
    // Bits 9-13: L1 (5 bits)
    let l1 = unpack_bits(&bits, &mut bit_idx, 5) as u8;
    
    // Bits 14-18: L2 (5 bits)
    let l2 = unpack_bits(&bits, &mut bit_idx, 5) as u8;
    
    // First subframe
    // Bits 19-26: P1 (8 bits)
    let p1 = unpack_bits(&bits, &mut bit_idx, 8) as u8;
    
    // Bit 27: Parity (1 bit)
    let _parity1 = bits[bit_idx];
    bit_idx += 1;
    
    // Bits 28-40: CB1 positions (13 bits)
    let c1 = unpack_bits(&bits, &mut bit_idx, 13);
    
    // Bits 41-44: S1 signs (4 bits)
    let s1 = unpack_bits(&bits, &mut bit_idx, 4);
    
    // Bits 45-47: GA1 (3 bits)
    let ga1 = unpack_bits(&bits, &mut bit_idx, 3) as u8;
    
    // Bits 48-51: GB1 (4 bits)
    let gb1 = unpack_bits(&bits, &mut bit_idx, 4) as u8;
    
    // Second subframe
    // Bits 52-56: P2 (5 bits)
    let p2 = unpack_bits(&bits, &mut bit_idx, 5) as u8;
    
    // Bits 57-69: CB2 positions (13 bits)
    let c2 = unpack_bits(&bits, &mut bit_idx, 13);
    
    // Bits 70-73: S2 signs (4 bits)
    let s2 = unpack_bits(&bits, &mut bit_idx, 4);
    
    // Bits 74-76: GA2 (3 bits)
    let ga2 = unpack_bits(&bits, &mut bit_idx, 3) as u8;
    
    // Bits 77-80: GB2 (4 bits)
    let gb2 = unpack_bits(&bits, &mut bit_idx, 4) as u8;
    
    // Reconstruct algebraic codebook indices
    let c1_full = encode_algebraic_index(s1, c1);
    let c2_full = encode_algebraic_index(s2, c2);
    
    // Note: L3 is not transmitted in G.729A
    let l3 = 0;  // Default value
    
    DecodedParameters {
        lsp_indices: [l0, l1, l2, l3],
        pitch_delays: [
            p1 as f32,  // First subframe absolute
            p2 as f32,  // Second subframe relative
        ],
        fixed_codebook_indices: [c1_full, c2_full],
        gain_indices: [[ga1, gb1], [ga2, gb2]],
    }
}

/// Pack bits into bit array
fn pack_bits(bits: &mut [u8], bit_idx: &mut usize, value: u32, num_bits: usize) {
    for i in 0..num_bits {
        if *bit_idx < bits.len() {
            bits[*bit_idx] = ((value >> (num_bits - 1 - i)) & 1) as u8;
            *bit_idx += 1;
        }
    }
}

/// Unpack bits from bit array
fn unpack_bits(bits: &[u8], bit_idx: &mut usize, num_bits: usize) -> u32 {
    let mut value = 0u32;
    for _ in 0..num_bits {
        if *bit_idx < bits.len() {
            value = (value << 1) | (bits[*bit_idx] as u32);
            *bit_idx += 1;
        } else {
            value = value << 1;  // Pad with zeros if we run out
        }
    }
    value
}

/// Compute parity bit for pitch delay (sum of 6 MSBs of P1)
fn compute_pitch_parity(pitch: u8) -> u8 {
    let mut parity = 0u8;
    for i in 2..8 {  // 6 MSBs
        parity ^= (pitch >> i) & 1;
    }
    parity
}

/// Decode 17-bit algebraic codebook index into signs (4 bits) and positions (13 bits)
fn decode_algebraic_index(index: u32) -> (u32, u32) {
    // Simplified - actual implementation is more complex
    let signs = (index >> 13) & 0xF;
    let positions = index & 0x1FFF;
    (signs, positions)
}

/// Encode signs and positions into 17-bit algebraic codebook index
fn encode_algebraic_index(signs: u32, positions: u32) -> u32 {
    (signs << 13) | (positions & 0x1FFF)
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_bitstream_pack_unpack() {
        // Create test parameters
        let params = EncodedFrame {
            lsp_indices: [63, 15, 12, 8],  // Test LSP indices
            pitch_delay_int: [60, 4],       // Test pitch delays
            pitch_delay_frac: [0, 1],       // Test fractional delays
            fixed_codebook_idx: [0x12345, 0x23456], // Test fixed codebook
            gain_indices: [[5, 10], [3, 7]], // Test gains
        };
        
        // Pack frame
        let packed = pack_frame(&params);
        
        // Unpack frame
        let unpacked = unpack_frame(&packed);
        
        // Check LSP indices
        assert_eq!(unpacked.lsp_indices[0], params.lsp_indices[0]);
        assert_eq!(unpacked.lsp_indices[1], params.lsp_indices[1]);
        assert_eq!(unpacked.lsp_indices[2], params.lsp_indices[2]);
        assert_eq!(unpacked.lsp_indices[3], params.lsp_indices[3]);
        
        // Check gains match
        assert_eq!(unpacked.gain_indices[0][0], params.gain_indices[0][0]);
        assert_eq!(unpacked.gain_indices[0][1], params.gain_indices[0][1]);
        assert_eq!(unpacked.gain_indices[1][0], params.gain_indices[1][0]);
        assert_eq!(unpacked.gain_indices[1][1], params.gain_indices[1][1]);
    }
    
    #[test]
    fn test_pitch_parity() {
        // Test parity calculation
        assert_eq!(compute_pitch_parity(0b11111100), 0); // Even number of 1s in MSBs
        assert_eq!(compute_pitch_parity(0b11111000), 1); // Odd number of 1s in MSBs
        assert_eq!(compute_pitch_parity(0b10101000), 1); // Odd number
        assert_eq!(compute_pitch_parity(0b11001100), 0); // Even number
    }
} 