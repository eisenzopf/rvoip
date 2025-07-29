use std::fs::File;
use std::io::{Read, Write};
use std::env;

// Import the actual G.729A encoder from the library
use g729a_new::common::basic_operators::Word16;
use g729a_new::encoder::g729a_encoder::{G729AEncoder, export_prm2bits as prm2bits, EXPORT_PRM_SIZE as PRM_SIZE, EXPORT_SERIAL_SIZE as SERIAL_SIZE};

// G.729A constants
const L_FRAME: usize = 80;      // Frame size (10ms at 8kHz)


/// Encode a complete audio file
fn encode_file(input_path: &str, output_path: &str) -> Result<(), Box<dyn std::error::Error>> {
    println!("Rust G.729A Encoder Test");
    println!("Input:  {}", input_path);
    println!("Output: {}", output_path);
    
    let mut input_file = File::open(input_path)?;
    let mut output_file = File::create(output_path)?;
    
    let mut encoder = G729AEncoder::new();
    encoder.init();
    
    let mut frame_count = 0;
    let mut buffer = [0u8; L_FRAME * 2]; // 2 bytes per Word16
    
    // Process file frame by frame
    while input_file.read_exact(&mut buffer).is_ok() {
        // Convert bytes to Word16 samples (little-endian)
        let mut speech_frame = [0i16; L_FRAME];
        for i in 0..L_FRAME {
            speech_frame[i] = i16::from_le_bytes([buffer[i*2], buffer[i*2+1]]);
        }
        
        // Encode the frame
        if frame_count == 0 {
            println!("Encoding first frame...");
        }
        let prm = encoder.encode_frame(&speech_frame);
        
        if frame_count == 0 {
            println!("First frame encoded, converting to bits...");
        }
        
        // Convert parameters to serial bits (fix parameter array structure if needed)
        let serial = prm2bits(&prm);
        
        if frame_count == 0 {
            println!("First frame bits: {:?}", &serial[0..10]);
        }
        
        // Write serial bits as bytes (compatible with C reference format)
        for &bit_word in &serial {
            output_file.write_all(&bit_word.to_le_bytes())?;
        }
        
        frame_count += 1;
        
        if frame_count % 100 == 0 {
            println!("Processed {} frames", frame_count);
        }
    }
    
    println!("Encoding complete: {} frames processed", frame_count);
    println!("Output size: {} bytes", frame_count * SERIAL_SIZE * 2);
    
    Ok(())
}

fn main() {
    let args: Vec<String> = env::args().collect();
    
    if args.len() != 4 {
        eprintln!("Usage: {} encode <input.pcm> <output.bit>", args[0]);
        eprintln!("       (only encoding is supported - decoder not yet implemented)");
        std::process::exit(1);
    }
    
    let operation = &args[1];
    let input_file = &args[2];
    let output_file = &args[3];
    
    match operation.as_str() {
        "encode" => {
            if let Err(e) = encode_file(input_file, output_file) {
                eprintln!("Encoding failed: {}", e);
                std::process::exit(1);
            }
        }
        _ => {
            eprintln!("Error: Only 'encode' operation is supported");
            eprintln!("The Rust decoder is not yet implemented");
            std::process::exit(1);
        }
    }
}
