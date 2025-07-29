//! Standalone binary to run G.729A compliance tests

use codec_core::codecs::g729a::{G729AEncoder, G729ADecoder, AudioFrame};
use std::fs::File;
use std::io::Read;
use std::path::Path;

// Re-implement the necessary functions here for standalone usage
fn read_pcm_file(path: &Path) -> Result<Vec<i16>, std::io::Error> {
    let mut file = File::open(path)?;
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer)?;
    
    let samples: Vec<i16> = buffer.chunks_exact(2)
        .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
        .collect();
    
    Ok(samples)
}

fn calculate_snr(original: &[i16], processed: &[i16]) -> f64 {
    if original.len() != processed.len() {
        return 0.0;
    }
    
    let signal_power: f64 = original.iter()
        .map(|&s| (s as f64) * (s as f64))
        .sum::<f64>() / original.len() as f64;
    
    let noise_power: f64 = original.iter()
        .zip(processed.iter())
        .map(|(&o, &p)| {
            let diff = (o as f64) - (p as f64);
            diff * diff
        })
        .sum::<f64>() / original.len() as f64;
    
    if noise_power > 0.0 {
        10.0 * (signal_power / noise_power).log10()
    } else {
        f64::INFINITY
    }
}

fn main() {
    println!("G.729A Compliance Test Runner");
    println!("=============================\n");
    
    let test_vectors_dir = Path::new("crates/codec-core/src/codecs/g729a/tests/test_vectors");
    
    // Quick test with ALGTHM vector
    let input_path = test_vectors_dir.join("ALGTHM.IN");
    let output_path = test_vectors_dir.join("ALGTHM.PST");
    
    if !input_path.exists() {
        eprintln!("Test vectors not found at {:?}", test_vectors_dir);
        eprintln!("Please ensure test vectors are in the correct location.");
        return;
    }
    
    println!("Running quick compliance check with ALGTHM test vector...\n");
    
    // Read input samples
    let input_samples = match read_pcm_file(&input_path) {
        Ok(samples) => samples,
        Err(e) => {
            eprintln!("Failed to read input file: {}", e);
            return;
        }
    };
    
    // Read reference output
    let reference_output = match read_pcm_file(&output_path) {
        Ok(samples) => samples,
        Err(e) => {
            eprintln!("Failed to read reference output: {}", e);
            return;
        }
    };
    
    println!("Input samples: {}", input_samples.len());
    println!("Reference output samples: {}", reference_output.len());
    println!("Frames to process: {}\n", input_samples.len() / 80);
    
    // Process through codec
    let mut encoder = G729AEncoder::new();
    let mut decoder = G729ADecoder::new();
    let mut output_samples = Vec::new();
    
    for (frame_idx, chunk) in input_samples.chunks(80).enumerate() {
        if chunk.len() < 80 {
            break;
        }
        
        let mut samples = [0i16; 80];
        samples.copy_from_slice(&chunk[..80]);
        
        let frame = AudioFrame {
            samples,
            timestamp: frame_idx as u64 * 80,
        };
        
        // Get lookahead
        let lookahead = if input_samples.len() > (frame_idx + 1) * 80 + 40 {
            let mut la = [0i16; 40];
            la.copy_from_slice(&input_samples[(frame_idx + 1) * 80..(frame_idx + 1) * 80 + 40]);
            la
        } else {
            [0i16; 40]
        };
        
        // Encode and decode
        match encoder.encode_frame_with_lookahead(&frame, &lookahead) {
            Ok(encoded) => {
                match decoder.decode_frame(&encoded) {
                    Ok(decoded) => {
                        output_samples.extend_from_slice(&decoded.samples);
                    }
                    Err(e) => {
                        eprintln!("Decoding error at frame {}: {:?}", frame_idx, e);
                        break;
                    }
                }
            }
            Err(e) => {
                eprintln!("Encoding error at frame {}: {:?}", frame_idx, e);
                break;
            }
        }
        
        // Print progress for long files
        if frame_idx > 0 && frame_idx % 100 == 0 {
            print!(".");
            use std::io::Write;
            std::io::stdout().flush().unwrap();
        }
    }
    println!("\n");
    
    // Compare outputs
    let samples_to_compare = output_samples.len().min(reference_output.len());
    
    if samples_to_compare == 0 {
        eprintln!("No samples to compare!");
        return;
    }
    
    // Calculate metrics
    let mut max_diff = 0i16;
    let mut total_diff = 0i64;
    let mut matching_samples = 0;
    
    for i in 0..samples_to_compare {
        let diff = (output_samples[i] as i32 - reference_output[i] as i32).abs() as i16;
        max_diff = max_diff.max(diff);
        total_diff += diff as i64;
        
        if output_samples[i] == reference_output[i] {
            matching_samples += 1;
        }
    }
    
    let avg_diff = total_diff as f64 / samples_to_compare as f64;
    let match_percentage = (matching_samples as f64 / samples_to_compare as f64) * 100.0;
    
    // Calculate SNR
    let snr = calculate_snr(&reference_output[..samples_to_compare], 
                           &output_samples[..samples_to_compare]);
    
    // Print results
    println!("Compliance Test Results");
    println!("======================");
    println!("Samples compared: {}", samples_to_compare);
    println!("Bit-exact matches: {} ({:.2}%)", matching_samples, match_percentage);
    println!("Maximum difference: {}", max_diff);
    println!("Average difference: {:.2}", avg_diff);
    println!("Signal-to-Noise Ratio: {:.2} dB", snr);
    
    println!("\nCompliance Score:");
    if match_percentage == 100.0 {
        println!("✅ PERFECT - Bit-exact match!");
    } else if snr > 30.0 {
        println!("✓ EXCELLENT - Very high quality (SNR > 30dB)");
    } else if snr > 20.0 {
        println!("✓ GOOD - Acceptable quality (SNR > 20dB)");
    } else if snr > 10.0 {
        println!("⚠️  FAIR - Needs improvement (SNR > 10dB)");
    } else {
        println!("❌ POOR - Significant issues (SNR < 10dB)");
    }
    
    println!("\nTo run full compliance tests, use:");
    println!("  cargo test --test compliance_tests -- --nocapture");
}

// Note: The main function above is the entry point 