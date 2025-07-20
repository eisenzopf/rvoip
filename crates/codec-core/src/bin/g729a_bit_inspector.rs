//! G.729A .BIT file inspector
//! 
//! This tool inspects ITU-T G.729A test vector .BIT files to understand
//! their format and help with compliance testing.

use std::fs::File;
use std::io::Read;
use std::path::Path;

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <path_to_bit_file>", args[0]);
        eprintln!("Example: {} crates/codec-core/src/codecs/g729a/tests/test_vectors/ALGTHM.BIT", args[0]);
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
    
    println!("G.729A .BIT File Inspector");
    println!("=========================");
    println!("File: {}", bit_file_path.display());
    println!("Size: {} bytes", buffer.len());
    println!("Expected frames: {}", buffer.len() / 10);
    println!();
    
    // G.729A uses 10 bytes (80 bits) per frame
    const FRAME_SIZE: usize = 10;
    
    // Inspect first few frames
    let frames_to_inspect = 5.min(buffer.len() / FRAME_SIZE);
    
    for frame_idx in 0..frames_to_inspect {
        let frame_start = frame_idx * FRAME_SIZE;
        let frame_end = frame_start + FRAME_SIZE;
        let frame_data = &buffer[frame_start..frame_end];
        
        println!("Frame {}: ", frame_idx);
        print!("  Hex: ");
        for byte in frame_data {
            print!("{:02X} ", byte);
        }
        println!();
        
        // Show binary representation of first few bytes
        print!("  Binary (first 3 bytes): ");
        for i in 0..3.min(frame_data.len()) {
            print!("{:08b} ", frame_data[i]);
        }
        println!();
        
        // Try to extract some key parameters (based on ITU-T G.729A spec)
        // The bit allocation is complex, but we can show the raw values
        if frame_data.len() >= 10 {
            // First byte contains part of L0 (first stage LSP index)
            let l0_partial = frame_data[0];
            println!("  First byte (L0 partial): 0x{:02X} = {}", l0_partial, l0_partial);
            
            // Show parameter bit positions according to ITU-T Table 8/G.729
            println!("  ITU-T G.729A Parameters (approximate extraction):");
            println!("    L0 (LSP 1st stage): bits 0-6 across bytes");
            println!("    L1,L2,L3 (LSP 2nd stage): distributed across frame");
            println!("    P1,P2 (pitch delays): distributed");
            println!("    C1,C2 (fixed codebook): distributed");
            println!("    GA1,GA2,GB1,GB2 (gains): distributed");
        }
        println!();
    }
    
    // Show last frame if there are many frames
    if buffer.len() / FRAME_SIZE > frames_to_inspect {
        let last_frame_idx = (buffer.len() / FRAME_SIZE) - 1;
        let frame_start = last_frame_idx * FRAME_SIZE;
        let frame_end = frame_start + FRAME_SIZE;
        let frame_data = &buffer[frame_start..frame_end];
        
        println!("...");
        println!("Last Frame {}: ", last_frame_idx);
        print!("  Hex: ");
        for byte in frame_data {
            print!("{:02X} ", byte);
        }
        println!();
    }
    
    // Check if file size is exact multiple of 10
    if buffer.len() % FRAME_SIZE != 0 {
        println!("\nWarning: File size is not a multiple of 10 bytes!");
        println!("Remaining bytes: {}", buffer.len() % FRAME_SIZE);
    }
} 