use g729a_new::G729AEncoder;
use g729a_new::G729ADecoder;
use g729a_new::common::tab_ld8a::L_FRAME;
use g729a_new::common::bits::{SERIAL_SIZE, prm2bits, bits2prm};
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

fn read_raw_samples(path: &Path) -> Vec<i16> {
    let mut file = File::open(path).expect("Failed to open file");
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer).expect("Failed to read file");
    
    // Skip WAV header (44 bytes)
    let samples_start = 44;
    let mut samples = Vec::new();
    
    for i in (samples_start..buffer.len()).step_by(2) {
        if i + 1 < buffer.len() {
            let sample = i16::from_le_bytes([buffer[i], buffer[i + 1]]);
            samples.push(sample);
        }
    }
    
    samples
}

fn write_debug_info(frame_idx: usize, prm: &[i16], bitstream: &[i16], decoded: &[i16]) {
    println!("\n=== Frame {} Debug Info ===", frame_idx);
    
    // Show parameters
    println!("Parameters (11 values):");
    println!("  LSP: L0={}, L1={}", prm[0], prm[1]);
    println!("  Subframe 1: Pitch={}, Parity={}, ACELP={}, Sign={}, Gain={}", 
             prm[2], prm[3], prm[4], prm[5], prm[6]);
    println!("  Subframe 2: Pitch={}, ACELP={}, Sign={}, Gain={}", 
             prm[7], prm[8], prm[9], prm[10]);
    
    // Check bitstream
    println!("Bitstream: sync=0x{:04x}, size={}", bitstream[0], bitstream[1]);
    
    // Check decoded output
    let energy: i64 = decoded.iter().map(|&x| (x as i64) * (x as i64)).sum();
    let avg_energy = energy / decoded.len() as i64;
    println!("Decoded: energy={}, avg_energy={}", energy, avg_energy);
    
    // Check for constant output
    let first = decoded[0];
    let all_same = decoded.iter().all(|&x| x == first);
    if all_same {
        println!("WARNING: All decoded samples are the same value: {}", first);
    }
    
    // Show first 10 samples
    print!("First 10 decoded samples: ");
    for i in 0..10.min(decoded.len()) {
        print!("{} ", decoded[i]);
    }
    println!();
}

#[test]
fn debug_codec_operation() {
    let test_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/round_trip");
    let input_path = test_dir.join("OSR_us_000_0010_8k.wav");
    
    // Read just the first few frames
    let samples = read_raw_samples(&input_path);
    println!("Loaded {} samples", samples.len());
    
    // Initialize codec
    let mut encoder = G729AEncoder::new();
    encoder.init();
    
    let mut decoder = G729ADecoder::new();
    decoder.init();
    
    // Process first 5 frames with debugging
    for frame_idx in 0..5 {
        let start = frame_idx * L_FRAME;
        let end = start + L_FRAME;
        
        if end > samples.len() {
            break;
        }
        
        let frame = &samples[start..end];
        
        // Show input
        let input_energy: i64 = frame.iter().map(|&x| (x as i64) * (x as i64)).sum();
        println!("\n--- Processing frame {} ---", frame_idx);
        println!("Input energy: {}", input_energy / L_FRAME as i64);
        
        // Encode
        let prm = encoder.encode_frame(frame);
        let bitstream = prm2bits(&prm);
        
        // Decode
        let decoded = decoder.decode_frame(&bitstream);
        
        // Debug info
        write_debug_info(frame_idx, &prm, &bitstream, &decoded);
        
        // Check round-trip
        let prm_check = bits2prm(&bitstream);
        let params_match = prm.iter().zip(prm_check.iter()).all(|(a, b)| a == b);
        println!("Parameter round-trip: {}", if params_match { "OK" } else { "FAILED" });
    }
}

#[test] 
fn analyze_decoded_pattern() {
    let test_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/round_trip");
    let decoded_path = test_dir.join("OSR_us_000_0010_8k_decoded.wav");
    
    let samples = read_raw_samples(&decoded_path);
    println!("Analyzing decoded file: {} samples", samples.len());
    
    // Check for repeating patterns
    let frame_size = L_FRAME;
    let num_frames = samples.len() / frame_size;
    
    println!("\nFirst 10 frames analysis:");
    for i in 0..10.min(num_frames) {
        let start = i * frame_size;
        let end = start + frame_size;
        let frame = &samples[start..end];
        
        let min = frame.iter().min().unwrap_or(&0);
        let max = frame.iter().max().unwrap_or(&0);
        let avg: i64 = frame.iter().map(|&x| x as i64).sum::<i64>() / frame_size as i64;
        
        // Check if constant
        let all_same = frame.iter().all(|&x| x == frame[0]);
        
        println!("Frame {}: min={}, max={}, avg={}, constant={}", 
                 i, min, max, avg, all_same);
        
        // Check for pure tone (sinusoidal pattern)
        if i == 0 {
            print!("  First 20 samples: ");
            for j in 0..20.min(frame.len()) {
                print!("{} ", frame[j]);
            }
            println!();
        }
    }
    
    // Frequency analysis - check for dominant frequency
    println!("\nChecking for periodic patterns...");
    let test_len = 1000.min(samples.len());
    
    // Simple autocorrelation to find period
    let mut max_corr = 0i64;
    let mut best_period = 0;
    
    for period in 10..200 {  // Check periods from 10 to 200 samples
        let mut corr = 0i64;
        let count = test_len - period;
        
        for i in 0..count {
            corr += (samples[i] as i64) * (samples[i + period] as i64);
        }
        
        if corr > max_corr {
            max_corr = corr;
            best_period = period;
        }
    }
    
    if best_period > 0 {
        let frequency = 8000.0 / best_period as f32;
        println!("Detected dominant period: {} samples ({:.1} Hz)", best_period, frequency);
    }
}

#[test]
fn analyze_encoder_output() {
    let test_dir = Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/round_trip");
    let input_path = test_dir.join("OSR_us_000_0010_8k.wav");
    
    let samples = read_raw_samples(&input_path);
    println!("Analyzing encoder output for {} samples", samples.len());
    
    let mut encoder = G729AEncoder::new();
    encoder.init();
    
    // Store parameters for multiple frames
    let mut all_params = Vec::new();
    
    // Process first 20 frames
    for frame_idx in 0..20.min(samples.len() / L_FRAME) {
        let start = frame_idx * L_FRAME;
        let end = start + L_FRAME;
        let frame = &samples[start..end];
        
        let prm = encoder.encode_frame(frame);
        all_params.push(prm);
    }
    
    // Analyze parameter patterns
    println!("\nParameter variation analysis:");
    
    // Check LSP parameters (indices 0 and 1)
    let lsp0_values: Vec<i16> = all_params.iter().map(|p| p[0]).collect();
    let lsp1_values: Vec<i16> = all_params.iter().map(|p| p[1]).collect();
    println!("LSP0 range: {} to {}", lsp0_values.iter().min().unwrap(), lsp0_values.iter().max().unwrap());
    println!("LSP1 range: {} to {}", lsp1_values.iter().min().unwrap(), lsp1_values.iter().max().unwrap());
    
    // Check pitch parameters for both subframes
    let pitch1_values: Vec<i16> = all_params.iter().map(|p| p[2]).collect();
    let pitch2_values: Vec<i16> = all_params.iter().map(|p| p[7]).collect();
    println!("Pitch1 range: {} to {}", pitch1_values.iter().min().unwrap(), pitch1_values.iter().max().unwrap());
    println!("Pitch2 delta range: {} to {}", pitch2_values.iter().min().unwrap(), pitch2_values.iter().max().unwrap());
    
    // Check ACELP indices
    let acelp1_values: Vec<i16> = all_params.iter().map(|p| p[4]).collect();
    let acelp2_values: Vec<i16> = all_params.iter().map(|p| p[8]).collect();
    println!("ACELP1 range: {} to {}", acelp1_values.iter().min().unwrap(), acelp1_values.iter().max().unwrap());
    println!("ACELP2 range: {} to {}", acelp2_values.iter().min().unwrap(), acelp2_values.iter().max().unwrap());
    
    // Print actual ACELP2 values to see the pattern
    println!("\nACELP2 values for first 10 frames:");
    for (i, val) in acelp2_values.iter().take(10).enumerate() {
        println!("  Frame {}: ACELP2={}", i, val);
    }
    
    // Check for stuck parameters
    println!("\nChecking for stuck parameters:");
    let param_names = ["LSP0", "LSP1", "Pitch1", "Parity", "ACELP1", "Sign1", "Gain1", 
                       "Pitch2", "ACELP2", "Sign2", "Gain2"];
    
    for (idx, name) in param_names.iter().enumerate() {
        let values: Vec<i16> = all_params.iter().map(|p| p[idx]).collect();
        let unique_count = values.iter().collect::<std::collections::HashSet<_>>().len();
        if unique_count == 1 {
            println!("  {} is STUCK at value {}", name, values[0]);
        } else if unique_count < 3 {
            println!("  {} has only {} unique values", name, unique_count);
        }
    }
    
    // Look for repeating patterns in parameters
    println!("\nChecking for repeating parameter patterns:");
    for period in 2..10 {
        let mut matches = true;
        if all_params.len() >= period * 2 {
            for i in 0..period {
                if all_params[i] != all_params[i + period] {
                    matches = false;
                    break;
                }
            }
            if matches {
                println!("  Found repeating pattern with period {} frames", period);
                break;
            }
        }
    }
}