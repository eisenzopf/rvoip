//! G.729A Bitstream Analyzer
//! 
//! Analyzes ITU-T G.729A bitstream structure according to Table 8/G.729

use std::fs::File;
use std::io::Read;
use std::path::Path;

// G.729A bit allocation according to ITU-T Table 8
// Total: 80 bits (10 bytes) per frame
struct G729AParameters {
    // LSP quantization (18 bits total)
    l0: u8,  // 1st stage LSP index (7 bits)
    l1: u8,  // 2nd stage lower LSP index (5 bits)
    l2: u8,  // 2nd stage lower LSP index (5 bits)
    l3: u8,  // 2nd stage upper LSP index (5 bits) - Only 1 bit in first subframe
    
    // Pitch delay - Subframe 1 (8 bits)
    p1: u8,
    
    // Pitch delay - Subframe 2 (5 bits)  
    p2: u8,
    
    // Fixed codebook - Subframe 1 (17 bits)
    c1: u32,
    
    // Fixed codebook - Subframe 2 (17 bits)
    c2: u32,
    
    // Gains - Subframe 1 (7 bits)
    ga1: u8, // Stage 1 gain (3 bits)
    gb1: u8, // Stage 2 gain (4 bits)
    
    // Gains - Subframe 2 (7 bits)
    ga2: u8, // Stage 1 gain (3 bits)
    gb2: u8, // Stage 2 gain (4 bits)
}

fn unpack_g729a_frame(frame: &[u8; 10]) -> G729AParameters {
    // This is a simplified unpacking - the actual bit packing is complex
    // The bits are distributed across bytes in a specific pattern
    
    // For now, just extract some key values to understand the pattern
    G729AParameters {
        l0: frame[0] >> 1,  // 7 bits from first byte
        l1: ((frame[0] & 0x01) << 4) | (frame[1] >> 4), // 5 bits
        l2: ((frame[1] & 0x0F) << 1) | (frame[2] >> 7), // 5 bits
        l3: (frame[2] >> 6) & 0x01, // 1 bit for first subframe
        
        p1: ((frame[2] & 0x3F) << 2) | (frame[3] >> 6), // 8 bits
        p2: (frame[3] >> 1) & 0x1F, // 5 bits
        
        // Fixed codebook indices are 17 bits each, spread across multiple bytes
        c1: 0, // TODO: Proper extraction
        c2: 0, // TODO: Proper extraction
        
        // Gain indices
        ga1: (frame[6] >> 4) & 0x07, // 3 bits
        gb1: frame[6] & 0x0F, // 4 bits
        ga2: (frame[8] >> 4) & 0x07, // 3 bits  
        gb2: frame[8] & 0x0F, // 4 bits
    }
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <path_to_bit_file>", args[0]);
        eprintln!("Example: {} src/codecs/g729a/tests/test_vectors/ALGTHM.BIT", args[0]);
        std::process::exit(1);
    }
    
    let bit_file_path = Path::new(&args[1]);
    if !bit_file_path.exists() {
        eprintln!("Error: File '{}' not found", bit_file_path.display());
        std::process::exit(1);
    }
    
    // Read the entire file
    let mut file = File::open(bit_file_path).expect("Failed to open file");
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).expect("Failed to read file");
    
    println!("G.729A Bitstream Analyzer");
    println!("========================");
    println!("File: {}", bit_file_path.display());
    println!("Size: {} bytes", buffer.len());
    println!("Frames: {}", buffer.len() / 10);
    println!();
    
    // Analyze patterns in the bitstream
    let mut l0_values = std::collections::HashMap::new();
    let mut byte_patterns = vec![std::collections::HashMap::new(); 10];
    
    for frame_data in buffer.chunks_exact(10) {
        let mut frame = [0u8; 10];
        frame.copy_from_slice(frame_data);
        
        let params = unpack_g729a_frame(&frame);
        *l0_values.entry(params.l0).or_insert(0) += 1;
        
        // Track byte patterns
        for (i, &byte) in frame.iter().enumerate() {
            *byte_patterns[i].entry(byte).or_insert(0) += 1;
        }
    }
    
    // Show first few frames in detail
    println!("First 5 frames:");
    for (i, frame_data) in buffer.chunks_exact(10).take(5).enumerate() {
        let mut frame = [0u8; 10];
        frame.copy_from_slice(frame_data);
        let params = unpack_g729a_frame(&frame);
        
        println!("Frame {}: L0={:3}, L1={:2}, L2={:2}, P1={:3}, P2={:2}", 
                 i, params.l0, params.l1, params.l2, params.p1, params.p2);
        print!("  Hex: ");
        for byte in frame {
            print!("{:02X} ", byte);
        }
        println!();
    }
    
    // Show LSP first stage distribution
    println!("\nLSP First Stage (L0) Distribution:");
    let mut l0_sorted: Vec<_> = l0_values.iter().collect();
    l0_sorted.sort_by_key(|&(k, _)| k);
    for (value, count) in l0_sorted.iter().take(10) {
        println!("  L0={:3}: {} times", value, count);
    }
    
    // Show common byte patterns
    println!("\nMost common byte values by position:");
    for (pos, patterns) in byte_patterns.iter().enumerate() {
        let mut sorted: Vec<_> = patterns.iter().collect();
        sorted.sort_by_key(|&(_, count)| std::cmp::Reverse(count));
        
        print!("  Byte {}: ", pos);
        for (value, count) in sorted.iter().take(3) {
            print!("0x{:02X}({}) ", value, count);
        }
        println!();
    }
} 