use std::path::PathBuf;
use std::fs::File;
use std::io::{Read, Write};
use std::time::Instant;

use bytes::Bytes;
use clap::Parser;
use media_core::{
    codec::{Codec, G711Codec, G711Variant},
    AudioBuffer, AudioFormat, SampleRate
};

/// Simple demo for G.711 codec
#[derive(Parser, Debug)]
#[command(author, version, about, long_about = None)]
struct Args {
    /// Operation mode (encode or decode)
    #[arg(short, long, default_value = "encode")]
    mode: String,

    /// G.711 variant (pcmu or pcma)
    #[arg(short, long, default_value = "pcmu")]
    variant: String,

    /// Input file (raw 16-bit PCM for encode, G.711 for decode)
    #[arg(short, long)]
    input: PathBuf,

    /// Output file 
    #[arg(short, long)]
    output: PathBuf,

    /// Sample rate for PCM data (Hz)
    #[arg(short, long, default_value = "8000")]
    sample_rate: u32,

    /// Print statistics
    #[arg(short, long)]
    stats: bool,
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    
    // Create the appropriate codec
    let variant = match args.variant.to_lowercase().as_str() {
        "pcmu" => G711Variant::PCMU,
        "pcma" => G711Variant::PCMA,
        _ => {
            eprintln!("Invalid G.711 variant: {}. Using PCMU.", args.variant);
            G711Variant::PCMU
        }
    };
    
    let codec = G711Codec::new(variant);
    println!("Using G.711 codec variant: {:?} ({})", variant, codec.name());
    
    // Read input file
    let mut input_file = File::open(&args.input)?;
    let mut input_data = Vec::new();
    input_file.read_to_end(&mut input_data)?;
    
    println!("Read {} bytes from {}", input_data.len(), args.input.display());
    
    // Process based on mode
    let start_time = Instant::now();
    let output_data = match args.mode.to_lowercase().as_str() {
        "encode" => {
            println!("Encoding 16-bit PCM to G.711 {}...", codec.name());
            
            // Create audio buffer with the right format
            let sample_rate = SampleRate::from_hz(args.sample_rate);
            let format = AudioFormat::mono_16bit(sample_rate);
            
            // Check if we have the expected number of bytes (2 per sample)
            if input_data.len() % 2 != 0 {
                eprintln!("Warning: Input data size is not a multiple of 2 bytes.");
                // Truncate to even length
                input_data.truncate(input_data.len() - (input_data.len() % 2));
            }
            
            let pcm_buffer = AudioBuffer::new(
                Bytes::from(input_data.clone()),
                format
            );
            
            // Encode the audio
            let encoded = codec.encode(&pcm_buffer)?;
            
            // Convert to Vec<u8> for file writing
            encoded.to_vec()
        },
        "decode" => {
            println!("Decoding G.711 {} to 16-bit PCM...", codec.name());
            
            // Decode the audio
            let decoded = codec.decode(&input_data)?;
            
            // Convert to Vec<u8> for file writing
            decoded.data.to_vec()
        },
        _ => {
            eprintln!("Invalid mode: {}. Must be 'encode' or 'decode'.", args.mode);
            return Ok(());
        }
    };
    
    let elapsed = start_time.elapsed();
    
    // Write output file
    let mut output_file = File::create(&args.output)?;
    output_file.write_all(&output_data)?;
    
    println!("Wrote {} bytes to {}", output_data.len(), args.output.display());
    
    // Print statistics if requested
    if args.stats {
        let ratio = if args.mode == "encode" {
            (output_data.len() as f64) / (input_data.len() as f64)
        } else {
            (input_data.len() as f64) / (output_data.len() as f64)
        };
        
        println!("Statistics:");
        println!("  Processing time: {:.2?}", elapsed);
        println!("  Input size: {} bytes", input_data.len());
        println!("  Output size: {} bytes", output_data.len());
        println!("  Compression ratio: {:.2}:1", 1.0 / ratio);
        
        // Calculate throughput
        let throughput = (input_data.len() as f64) / elapsed.as_secs_f64() / 1_000_000.0;
        println!("  Throughput: {:.2} MB/s", throughput);
    }
    
    Ok(())
} 