use crate::common::basic_operators::Word16;

// G.729A constants
pub const PRM_SIZE: usize = 11;      // Number of analysis parameters per frame
pub const SERIAL_SIZE: usize = 82;   // Serial frame size (80 bits + 2 sync)

// G.729A bit stream format constants
pub const BIT_0: Word16 = 0x007f;   // Zero bit in bitstream
pub const BIT_1: Word16 = 0x0081;   // One bit in bitstream
pub const SYNC_WORD: Word16 = 0x6b21;  // Frame sync word
pub const SIZE_WORD: Word16 = 80;      // Number of speech bits

/// Convert parameters to serial bitstream (G.729A bit allocation)
/// 
/// Bit allocation per frame (80 bits total):
/// - LSP indices: 18 bits (L0=10, L1=8)
/// - Subframe 1: P1=8, S1=1, C1=13, GA1=3, GB1=4 (29 bits)
/// - Subframe 2: P2=5, S2=1, C2=13, GA2=3, GB2=4 (26 bits)
/// - Parity: 1 bit
/// - Fixed codebook signs are embedded in the codebook indices
pub fn prm2bits(prm: &[Word16; PRM_SIZE]) -> [Word16; SERIAL_SIZE] {
    let mut serial = [0i16; SERIAL_SIZE];
    
    // Add sync word and size
    serial[0] = SYNC_WORD;
    serial[1] = SIZE_WORD;
    
    let mut bit_pos = 2;  // Start after sync and size words
    
    // LSP indices: 18 bits (L0=10, L1=8)
    pack_bits(&mut serial, &mut bit_pos, prm[0] as u16, 10);
    pack_bits(&mut serial, &mut bit_pos, prm[1] as u16, 8);
    
    // Subframe 1
    pack_bits(&mut serial, &mut bit_pos, prm[2] as u16, 8);    // P1: Pitch delay
    pack_bits(&mut serial, &mut bit_pos, prm[3] as u16, 1);    // S1: Pitch parity
    pack_bits(&mut serial, &mut bit_pos, prm[4] as u16, 13);   // C1: Fixed codebook
    pack_bits(&mut serial, &mut bit_pos, prm[5] as u16, 3);    // GA1: Adaptive gain
    pack_bits(&mut serial, &mut bit_pos, prm[6] as u16, 4);    // GB1: Fixed gain
    
    // Subframe 2
    pack_bits(&mut serial, &mut bit_pos, prm[7] as u16, 5);    // P2: Relative pitch
    pack_bits(&mut serial, &mut bit_pos, prm[8] as u16, 13);   // C2: Fixed codebook
    pack_bits(&mut serial, &mut bit_pos, prm[9] as u16, 3);    // GA2: Adaptive gain
    pack_bits(&mut serial, &mut bit_pos, prm[10] as u16, 4);   // GB2: Fixed gain
    
    // The fixed codebook sign bits S1 and S2 are included in C1 and C2
    
    serial
}

/// Unpack serial bitstream to parameters
pub fn bits2prm(serial: &[Word16; SERIAL_SIZE]) -> [Word16; PRM_SIZE] {
    let mut prm = [0i16; PRM_SIZE];
    let mut bit_pos = 2;  // Skip sync word and size word
    
    // LSP indices
    prm[0] = unpack_bits(serial, &mut bit_pos, 10) as i16;
    prm[1] = unpack_bits(serial, &mut bit_pos, 8) as i16;
    
    // Subframe 1
    prm[2] = unpack_bits(serial, &mut bit_pos, 8) as i16;     // P1: Pitch delay
    prm[3] = unpack_bits(serial, &mut bit_pos, 1) as i16;     // S1: Pitch parity
    prm[4] = unpack_bits(serial, &mut bit_pos, 13) as i16;    // C1: Fixed codebook
    prm[5] = unpack_bits(serial, &mut bit_pos, 3) as i16;     // GA1: Adaptive gain
    prm[6] = unpack_bits(serial, &mut bit_pos, 4) as i16;     // GB1: Fixed gain
    
    // Subframe 2
    prm[7] = unpack_bits(serial, &mut bit_pos, 5) as i16;     // P2: Relative pitch
    prm[8] = unpack_bits(serial, &mut bit_pos, 13) as i16;    // C2: Fixed codebook
    prm[9] = unpack_bits(serial, &mut bit_pos, 3) as i16;     // GA2: Adaptive gain
    prm[10] = unpack_bits(serial, &mut bit_pos, 4) as i16;    // GB2: Fixed gain
    
    prm
}

/// Pack bits into serial array
fn pack_bits(serial: &mut [Word16], bit_pos: &mut usize, value: u16, num_bits: usize) {
    for i in 0..num_bits {
        if *bit_pos < serial.len() {
            let bit = (value >> (num_bits - 1 - i)) & 1;
            serial[*bit_pos] = if bit == 1 { BIT_1 } else { BIT_0 };
            *bit_pos += 1;
        }
    }
}

/// Unpack bits from serial array
fn unpack_bits(serial: &[Word16], bit_pos: &mut usize, num_bits: usize) -> u16 {
    let mut value = 0u16;
    for _ in 0..num_bits {
        if *bit_pos < serial.len() {
            let bit = if serial[*bit_pos] == BIT_1 { 1 } else { 0 };
            value = (value << 1) | bit;
            *bit_pos += 1;
        }
    }
    value
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_bit_packing_roundtrip() {
        // Test parameters with various values
        let prm: [Word16; PRM_SIZE] = [
            1023,  // LSP L0 (10 bits max)
            255,   // LSP L1 (8 bits max)
            143,   // P1 (8 bits)
            1,     // S1 (1 bit)
            8191,  // C1 (13 bits max)
            7,     // GA1 (3 bits max)
            15,    // GB1 (4 bits max)
            31,    // P2 (5 bits max)
            4095,  // C2 (13 bits)
            3,     // GA2 (3 bits)
            7,     // GB2 (4 bits)
        ];
        
        // Pack to bits
        let serial = prm2bits(&prm);
        
        // Unpack back
        let prm_decoded = bits2prm(&serial);
        
        // Verify roundtrip
        assert_eq!(prm, prm_decoded);
    }
    
    #[test]
    fn test_bit_allocation() {
        let prm: [Word16; PRM_SIZE] = [0; PRM_SIZE];
        let serial = prm2bits(&prm);
        
        // Count actual bits used (should be 80)
        let mut bit_count = 0;
        bit_count += 10 + 8;        // LSP
        bit_count += 8 + 1 + 13 + 3 + 4;  // Subframe 1
        bit_count += 5 + 13 + 3 + 4;      // Subframe 2
        
        assert_eq!(bit_count, 80);
        
        // Serial size should accommodate all bits plus sync
        assert!(SERIAL_SIZE >= 80);
    }
}