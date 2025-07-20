//! Bitstream packing and unpacking for G.729A

use crate::codecs::g729a::types::{EncodedFrame, DecodedParameters, Q15};

/// Pack encoded parameters into 80-bit (10-byte) frame
pub fn pack_frame(params: &EncodedFrame) -> [u8; 10] {
    let mut packed = [0u8; 10];
    let mut bit_pos = 0;
    
    // LSP indices: 18 bits total
    // L0: 7 bits
    write_bits(&mut packed, &mut bit_pos, params.lsp_indices[0] as u32, 7);
    // L1: 5 bits
    write_bits(&mut packed, &mut bit_pos, params.lsp_indices[1] as u32, 5);
    // L2: 5 bits
    write_bits(&mut packed, &mut bit_pos, params.lsp_indices[2] as u32, 5);
    // L3: 1 bit (unused in actual implementation, set to 0)
    write_bits(&mut packed, &mut bit_pos, params.lsp_indices[3] as u32, 1);
    
    // Subframe 1
    // P1: 8 bits (pitch delay integer part)
    write_bits(&mut packed, &mut bit_pos, params.pitch_delay_int[0] as u32, 8);
    // P0: 1 bit (pitch delay fractional part - simplified)
    write_bits(&mut packed, &mut bit_pos, params.pitch_delay_frac[0] as u32, 1);
    // C1: 17 bits (fixed codebook)
    write_bits(&mut packed, &mut bit_pos, params.fixed_codebook_idx[0], 17);
    // GA1: 3 bits (adaptive gain)
    write_bits(&mut packed, &mut bit_pos, params.gain_indices[0][0] as u32, 3);
    // GB1: 4 bits (fixed gain)
    write_bits(&mut packed, &mut bit_pos, params.gain_indices[0][1] as u32, 4);
    
    // Subframe 2
    // P2: 5 bits (pitch delay differential)
    write_bits(&mut packed, &mut bit_pos, params.pitch_delay_int[1] as u32, 5);
    // P3: 2 bits (pitch delay fractional)
    write_bits(&mut packed, &mut bit_pos, params.pitch_delay_frac[1] as u32, 2);
    // C2: 17 bits (fixed codebook)
    write_bits(&mut packed, &mut bit_pos, params.fixed_codebook_idx[1], 17);
    // GA2: 3 bits (adaptive gain)
    write_bits(&mut packed, &mut bit_pos, params.gain_indices[1][0] as u32, 3);
    // GB2: 4 bits (fixed gain)
    write_bits(&mut packed, &mut bit_pos, params.gain_indices[1][1] as u32, 4);
    
    packed
}

/// Unpack 80-bit frame into decoded parameters
pub fn unpack_frame(packed: &[u8; 10]) -> DecodedParameters {
    let mut bit_pos = 0;
    
    // LSP indices
    let l0 = read_bits(packed, &mut bit_pos, 7) as u8;
    let l1 = read_bits(packed, &mut bit_pos, 5) as u8;
    let l2 = read_bits(packed, &mut bit_pos, 5) as u8;
    let l3 = read_bits(packed, &mut bit_pos, 1) as u8;
    
    // Subframe 1
    let p1 = read_bits(packed, &mut bit_pos, 8) as u8;
    let p0 = read_bits(packed, &mut bit_pos, 1) as u8;
    let c1 = read_bits(packed, &mut bit_pos, 17);
    let ga1 = read_bits(packed, &mut bit_pos, 3) as u8;
    let gb1 = read_bits(packed, &mut bit_pos, 4) as u8;
    
    // Subframe 2
    let p2 = read_bits(packed, &mut bit_pos, 5) as u8;
    let p3 = read_bits(packed, &mut bit_pos, 2) as u8;
    let c2 = read_bits(packed, &mut bit_pos, 17);
    let ga2 = read_bits(packed, &mut bit_pos, 3) as u8;
    let gb2 = read_bits(packed, &mut bit_pos, 4) as u8;
    
    // Convert to internal representation
    DecodedParameters {
        lsp_indices: [l0, l1, l2, l3],
        pitch_delays: [
            p1 as f32 + (p0 as f32 / 3.0),  // Simplified fractional
            p2 as f32 + (p3 as f32 / 3.0),
        ],
        fixed_codebook_indices: [c1, c2],
        gain_indices: [[ga1, gb1], [ga2, gb2]],
    }
}

/// Write bits to packed array
fn write_bits(packed: &mut [u8], bit_pos: &mut usize, value: u32, num_bits: usize) {
    for i in 0..num_bits {
        let bit = (value >> (num_bits - 1 - i)) & 1;
        let byte_idx = *bit_pos / 8;
        let bit_idx = 7 - (*bit_pos % 8);
        
        if byte_idx < packed.len() {
            if bit == 1 {
                packed[byte_idx] |= 1 << bit_idx;
            } else {
                packed[byte_idx] &= !(1 << bit_idx);
            }
        }
        
        *bit_pos += 1;
    }
}

/// Read bits from packed array
fn read_bits(packed: &[u8], bit_pos: &mut usize, num_bits: usize) -> u32 {
    let mut value = 0u32;
    
    for _ in 0..num_bits {
        let byte_idx = *bit_pos / 8;
        let bit_idx = 7 - (*bit_pos % 8);
        
        if byte_idx < packed.len() {
            let bit = (packed[byte_idx] >> bit_idx) & 1;
            value = (value << 1) | bit as u32;
        }
        
        *bit_pos += 1;
    }
    
    value
}

/// Compute parity bit for error detection (simplified)
pub fn compute_parity(indices: &[u8]) -> u8 {
    let mut parity = 0u8;
    for &idx in indices {
        parity ^= idx;
    }
    parity & 1
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pack_unpack_round_trip() {
        let params = EncodedFrame {
            lsp_indices: [63, 15, 15, 0],  // Within bit limits
            pitch_delay_int: [50, 5],       // 8 bits, 5 bits
            pitch_delay_frac: [1, 2],       // 1 bit, 2 bits
            fixed_codebook_idx: [12345, 54321],
            gain_indices: [[5, 10], [6, 11]],
        };
        
        let packed = pack_frame(&params);
        let unpacked = unpack_frame(&packed);
        
        // Check LSP indices
        assert_eq!(unpacked.lsp_indices[0], params.lsp_indices[0]);
        assert_eq!(unpacked.lsp_indices[1], params.lsp_indices[1]);
        assert_eq!(unpacked.lsp_indices[2], params.lsp_indices[2]);
        
        // Check codebook indices
        assert_eq!(unpacked.fixed_codebook_indices[0], params.fixed_codebook_idx[0]);
        assert_eq!(unpacked.fixed_codebook_indices[1], params.fixed_codebook_idx[1]);
        
        // Check gain indices
        assert_eq!(unpacked.gain_indices[0], params.gain_indices[0]);
        assert_eq!(unpacked.gain_indices[1], params.gain_indices[1]);
    }

    #[test]
    fn test_write_read_bits() {
        let mut packed = [0u8; 2];
        let mut write_pos = 0;
        
        // Write some values
        write_bits(&mut packed, &mut write_pos, 0b101, 3);
        write_bits(&mut packed, &mut write_pos, 0b1100, 4);
        write_bits(&mut packed, &mut write_pos, 0b1, 1);
        
        // Read them back
        let mut read_pos = 0;
        assert_eq!(read_bits(&packed, &mut read_pos, 3), 0b101);
        assert_eq!(read_bits(&packed, &mut read_pos, 4), 0b1100);
        assert_eq!(read_bits(&packed, &mut read_pos, 1), 0b1);
    }

    #[test]
    fn test_compute_parity() {
        let indices = [1, 2, 3, 4];
        let parity = compute_parity(&indices);
        
        // 1 ^ 2 ^ 3 ^ 4 = 4, 4 & 1 = 0
        assert_eq!(parity, 0);
        
        let indices2 = [1, 2, 3, 5];
        let parity2 = compute_parity(&indices2);
        
        // 1 ^ 2 ^ 3 ^ 5 = 5, 5 & 1 = 1
        assert_eq!(parity2, 1);
    }

    #[test]
    fn test_packed_size() {
        let params = EncodedFrame {
            lsp_indices: [0; 4],
            pitch_delay_int: [0; 2],
            pitch_delay_frac: [0; 2],
            fixed_codebook_idx: [0; 2],
            gain_indices: [[0; 2]; 2],
        };
        
        let packed = pack_frame(&params);
        assert_eq!(packed.len(), 10); // 80 bits = 10 bytes
    }
} 