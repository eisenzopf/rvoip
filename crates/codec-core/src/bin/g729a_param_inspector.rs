//! G.729A Parameter Inspector
//! 
//! Decodes and displays parameters from G.729A bitstreams

use codec_core::codecs::g729a::codec::bitstream::{unpack_frame};
use codec_core::codecs::g729a::bitstream_utils::read_itu_bitstream;
use std::path::Path;

fn display_parameters(frame_data: &[u8; 10], frame_idx: usize) {
    let params = unpack_frame(frame_data);
    
    println!("Frame {} parameters:", frame_idx);
    println!("  Hex: {:02X?}", frame_data);
    
    // Display in binary to see bit patterns
    print!("  Binary: ");
    for i in 0..3 {
        print!("{:08b} ", frame_data[i]);
    }
    println!("...");
    
    println!("  LSP indices: L0={}, L1={}, L2={}, L3={}", 
             params.lsp_indices[0], params.lsp_indices[1], 
             params.lsp_indices[2], params.lsp_indices[3]);
    
    println!("  Pitch delays: P1={:.0}, P2={:.0}", 
             params.pitch_delays[0], params.pitch_delays[1]);
    
    println!("  Fixed CB indices: C1=0x{:05X}, C2=0x{:05X}", 
             params.fixed_codebook_indices[0], params.fixed_codebook_indices[1]);
    
    println!("  Gain indices: [GA1={}, GB1={}], [GA2={}, GB2={}]",
             params.gain_indices[0][0], params.gain_indices[0][1],
             params.gain_indices[1][0], params.gain_indices[1][1]);
    println!();
}

fn main() {
    let args: Vec<String> = std::env::args().collect();
    if args.len() < 2 {
        eprintln!("Usage: {} <path_to_bit_file>", args[0]);
        eprintln!("Example: {} src/codecs/g729a/tests/test_vectors/ALGTHM.BIT", args[0]);
        std::process::exit(1);
    }
    
    let bit_file_path = Path::new(&args[1]);
    
    // Read ITU-T format bitstream
    let frames = read_itu_bitstream(&bit_file_path)
        .expect("Failed to read bitstream");
    
    println!("G.729A Parameter Inspector");
    println!("=========================");
    println!("File: {}", bit_file_path.display());
    println!("Total frames: {}\n", frames.len());
    
    // Display first few frames
    for i in 0..5.min(frames.len()) {
        display_parameters(&frames[i], i);
    }
    
    // Analyze patterns
    if frames.len() > 10 {
        println!("Parameter Statistics:");
        println!("-------------------");
        
        // L0 distribution
        let mut l0_counts = std::collections::HashMap::new();
        for frame in &frames {
            let params = unpack_frame(frame);
            *l0_counts.entry(params.lsp_indices[0]).or_insert(0) += 1;
        }
        
        print!("L0 values: ");
        let mut l0_sorted: Vec<_> = l0_counts.iter().collect();
        l0_sorted.sort_by_key(|&(k, _)| k);
        for (val, count) in l0_sorted.iter().take(5) {
            print!("{}({}) ", val, count);
        }
        println!();
        
        // Pitch range
        let mut min_pitch: f32 = 255.0;
        let mut max_pitch: f32 = 0.0;
        for frame in &frames {
            let params = unpack_frame(frame);
            min_pitch = min_pitch.min(params.pitch_delays[0]);
            max_pitch = max_pitch.max(params.pitch_delays[0]);
        }
        println!("Pitch range: {:.0} - {:.0}", min_pitch, max_pitch);
    }
} 