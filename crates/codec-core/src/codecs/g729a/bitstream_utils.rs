//! Utilities for handling ITU-T G.729A test vector bitstream format
//!
//! The ITU-T test vectors use an expanded format where each bit is stored
//! as a 16-bit word: 0x007F for '0' and 0x0081 for '1'

use std::fs::File;
use std::io::Read;
use std::path::Path;

const SYNC_WORD: u16 = 0x6b21;
const BIT_0: u16 = 0x007f;
const BIT_1: u16 = 0x0081;
const FRAME_SIZE: u16 = 80;

/// Read ITU-T expanded bitstream format and convert to packed frames
pub fn read_itu_bitstream(path: &Path) -> Result<Vec<[u8; 10]>, std::io::Error> {
    let mut file = File::open(path)?;
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer)?;
    
    let mut frames = Vec::new();
    let mut pos = 0;
    
    // Process each frame (82 16-bit words)
    while pos + 164 <= buffer.len() {  // 82 words * 2 bytes = 164 bytes
        // Read sync word
        let sync = u16::from_le_bytes([buffer[pos], buffer[pos + 1]]);
        if sync != SYNC_WORD {
            eprintln!("Warning: Invalid sync word 0x{:04X} at position {}", sync, pos);
            pos += 2;
            continue;
        }
        pos += 2;
        
        // Read frame size
        let size = u16::from_le_bytes([buffer[pos], buffer[pos + 1]]);
        if size != FRAME_SIZE {
            eprintln!("Warning: Invalid frame size {} at position {}", size, pos);
            pos += 2;
            continue;
        }
        pos += 2;
        
        // Read 80 bits (80 16-bit words)
        let mut packed_frame = [0u8; 10];
        let mut bit_count = 0;
        
        for _ in 0..80 {
            let word = u16::from_le_bytes([buffer[pos], buffer[pos + 1]]);
            pos += 2;
            
            let bit = match word {
                BIT_0 => 0,
                BIT_1 => 1,
                _ => {
                    eprintln!("Warning: Invalid bit word 0x{:04X}", word);
                    0
                }
            };
            
            // Pack bit into frame
            let byte_idx = bit_count / 8;
            let bit_idx = 7 - (bit_count % 8);  // MSB first
            if bit == 1 {
                packed_frame[byte_idx] |= 1 << bit_idx;
            }
            bit_count += 1;
        }
        
        frames.push(packed_frame);
    }
    
    Ok(frames)
}

/// Convert packed frame to ITU-T expanded format
pub fn frame_to_itu_format(frame: &[u8; 10]) -> Vec<u16> {
    let mut words = Vec::with_capacity(82);
    
    // Add sync word and frame size
    words.push(SYNC_WORD);
    words.push(FRAME_SIZE);
    
    // Convert each bit to a word
    for byte in frame {
        for bit_idx in (0..8).rev() {  // MSB first
            let bit = (byte >> bit_idx) & 1;
            words.push(if bit == 1 { BIT_1 } else { BIT_0 });
        }
    }
    
    words
}

/// Write frames in ITU-T expanded format
pub fn write_itu_bitstream(path: &Path, frames: &[[u8; 10]]) -> Result<(), std::io::Error> {
    use std::io::Write;
    
    let mut file = File::create(path)?;
    
    for frame in frames {
        let words = frame_to_itu_format(frame);
        for word in words {
            file.write_all(&word.to_le_bytes())?;
        }
    }
    
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_bit_packing() {
        // Test frame with known pattern
        let frame = [0xFF, 0x00, 0xAA, 0x55, 0x0F, 0xF0, 0x81, 0x7E, 0x00, 0xFF];
        let words = frame_to_itu_format(&frame);
        
        assert_eq!(words.len(), 82);
        assert_eq!(words[0], SYNC_WORD);
        assert_eq!(words[1], FRAME_SIZE);
        
        // Check first byte (0xFF = all 1s)
        for i in 2..10 {
            assert_eq!(words[i], BIT_1);
        }
        
        // Check second byte (0x00 = all 0s)
        for i in 10..18 {
            assert_eq!(words[i], BIT_0);
        }
    }
} 