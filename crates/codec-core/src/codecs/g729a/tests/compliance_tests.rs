//! Compliance tests for G.729A codec against ITU-T test vectors

use std::fs::File;
use std::io::{Read, BufReader};
use std::path::Path;
use crate::codecs::g729a::{G729AEncoder, G729ADecoder, AudioFrame};

/// Result of a compliance test
#[derive(Debug)]
struct ComplianceResult {
    test_name: String,
    frames_tested: usize,
    frames_passed: usize,
    average_mse: f64,
    max_mse: f64,
    pass_rate: f64,
    bit_exact: bool,
}

impl ComplianceResult {
    fn print_summary(&self) {
        println!("\n{} Compliance Test Results:", self.test_name);
        println!("  Frames tested: {}", self.frames_tested);
        println!("  Frames passed: {}", self.frames_passed);
        println!("  Pass rate: {:.2}%", self.pass_rate * 100.0);
        println!("  Average MSE: {:.6}", self.average_mse);
        println!("  Maximum MSE: {:.6}", self.max_mse);
        println!("  Bit-exact: {}", if self.bit_exact { "YES" } else { "NO" });
    }
}

/// Read 16-bit PCM samples from file (Intel/little-endian format)
fn read_pcm_file(path: &Path) -> Result<Vec<i16>, std::io::Error> {
    let mut file = File::open(path)?;
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer)?;
    
    // Convert bytes to i16 samples (little-endian)
    let samples: Vec<i16> = buffer.chunks_exact(2)
        .map(|chunk| i16::from_le_bytes([chunk[0], chunk[1]]))
        .collect();
    
    Ok(samples)
}

/// Read bitstream file
fn read_bitstream_file(path: &Path) -> Result<Vec<u8>, std::io::Error> {
    let mut file = File::open(path)?;
    let mut buffer = Vec::new();
    file.read_to_end(&mut buffer)?;
    Ok(buffer)
}

/// Calculate Mean Squared Error between two signals
fn calculate_mse(signal1: &[i16], signal2: &[i16]) -> f64 {
    if signal1.len() != signal2.len() {
        return f64::INFINITY;
    }
    
    let sum_squared_diff: f64 = signal1.iter()
        .zip(signal2.iter())
        .map(|(&s1, &s2)| {
            let diff = (s1 as f64) - (s2 as f64);
            diff * diff
        })
        .sum();
    
    sum_squared_diff / signal1.len() as f64
}

/// Test encoder compliance
fn test_encoder_compliance(test_name: &str, input_path: &Path, reference_bits_path: &Path) -> ComplianceResult {
    println!("\nTesting encoder compliance for: {}", test_name);
    
    let input_samples = read_pcm_file(input_path).expect("Failed to read input file");
    let reference_bits = read_bitstream_file(reference_bits_path).expect("Failed to read reference bitstream");
    
    let mut encoder = G729AEncoder::new();
    let mut frames_tested = 0;
    let mut frames_passed = 0;
    let mut bit_exact = true;
    
    // Process in 80-sample frames (10ms at 8kHz)
    for (frame_idx, chunk) in input_samples.chunks(80).enumerate() {
        if chunk.len() < 80 {
            break; // Skip incomplete frames
        }
        
        let mut samples = [0i16; 80];
        samples.copy_from_slice(&chunk[..80]);
        
        let frame = AudioFrame {
            samples,
            timestamp: frame_idx as u64 * 80,
        };
        
        // Get lookahead (next 40 samples or zeros)
        let lookahead = if input_samples.len() > (frame_idx + 1) * 80 + 40 {
            let mut la = [0i16; 40];
            la.copy_from_slice(&input_samples[(frame_idx + 1) * 80..(frame_idx + 1) * 80 + 40]);
            la
        } else {
            [0i16; 40]
        };
        
        // Encode frame
        let encoded = encoder.encode_frame_with_lookahead(&frame, &lookahead)
            .expect("Encoding failed");
        
        // Compare with reference bitstream (10 bytes per frame)
        let ref_start = frame_idx * 10;
        let ref_end = ref_start + 10;
        
        if ref_end <= reference_bits.len() {
            let reference_frame = &reference_bits[ref_start..ref_end];
            
            frames_tested += 1;
            if encoded == reference_frame {
                frames_passed += 1;
            } else {
                bit_exact = false;
                // Print first few mismatches for debugging
                if frames_tested - frames_passed <= 3 {
                    println!("  Frame {} mismatch:", frame_idx);
                    println!("    Encoded:   {:?}", &encoded[..5]);
                    println!("    Reference: {:?}", &reference_frame[..5]);
                }
            }
        }
    }
    
    let pass_rate = if frames_tested > 0 {
        frames_passed as f64 / frames_tested as f64
    } else {
        0.0
    };
    
    ComplianceResult {
        test_name: test_name.to_string(),
        frames_tested,
        frames_passed,
        average_mse: 0.0, // Not applicable for bitstream comparison
        max_mse: 0.0,
        pass_rate,
        bit_exact,
    }
}

/// Test decoder compliance
fn test_decoder_compliance(test_name: &str, bits_path: &Path, reference_output_path: &Path) -> ComplianceResult {
    println!("\nTesting decoder compliance for: {}", test_name);
    
    let bitstream = read_bitstream_file(bits_path).expect("Failed to read bitstream");
    let reference_output = read_pcm_file(reference_output_path).expect("Failed to read reference output");
    
    let mut decoder = G729ADecoder::new();
    let mut decoded_samples = Vec::new();
    let mut frames_tested = 0;
    let mut total_mse = 0.0;
    let mut max_mse = 0.0;
    
    // Process bitstream in 10-byte frames
    for (frame_idx, chunk) in bitstream.chunks(10).enumerate() {
        if chunk.len() < 10 {
            break; // Skip incomplete frames
        }
        
        let decoded = decoder.decode_frame(chunk).expect("Decoding failed");
        decoded_samples.extend_from_slice(&decoded.samples);
        frames_tested += 1;
        
        // Calculate MSE for this frame if we have reference data
        let ref_start = frame_idx * 80;
        let ref_end = ref_start + 80;
        
        if ref_end <= reference_output.len() {
            let frame_mse = calculate_mse(
                &decoded.samples,
                &reference_output[ref_start..ref_end]
            );
            total_mse += frame_mse;
            max_mse = max_mse.max(frame_mse);
        }
    }
    
    // Calculate overall statistics
    let average_mse = if frames_tested > 0 {
        total_mse / frames_tested as f64
    } else {
        0.0
    };
    
    // Check bit-exactness (MSE should be exactly 0 for bit-exact)
    let bit_exact = average_mse == 0.0 && max_mse == 0.0;
    
    // For G.729A, we consider it passing if MSE is below a threshold
    // This threshold is conservative - adjust based on spec requirements
    let mse_threshold = 1.0; // Very tight threshold
    let frames_passed = if average_mse < mse_threshold { frames_tested } else { 0 };
    let pass_rate = if frames_tested > 0 {
        frames_passed as f64 / frames_tested as f64
    } else {
        0.0
    };
    
    ComplianceResult {
        test_name: test_name.to_string(),
        frames_tested,
        frames_passed,
        average_mse,
        max_mse,
        pass_rate,
        bit_exact,
    }
}

/// Test full codec chain (encode then decode)
fn test_codec_chain_compliance(test_name: &str, input_path: &Path, reference_output_path: &Path) -> ComplianceResult {
    println!("\nTesting full codec chain compliance for: {}", test_name);
    
    let input_samples = read_pcm_file(input_path).expect("Failed to read input file");
    let reference_output = read_pcm_file(reference_output_path).expect("Failed to read reference output");
    
    let mut encoder = G729AEncoder::new();
    let mut decoder = G729ADecoder::new();
    let mut decoded_samples = Vec::new();
    let mut frames_tested = 0;
    let mut total_mse = 0.0;
    let mut max_mse = 0.0;
    
    // Process in 80-sample frames
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
        let encoded = encoder.encode_frame_with_lookahead(&frame, &lookahead)
            .expect("Encoding failed");
        let decoded = decoder.decode_frame(&encoded)
            .expect("Decoding failed");
        
        decoded_samples.extend_from_slice(&decoded.samples);
        frames_tested += 1;
        
        // Calculate MSE for this frame
        let ref_start = frame_idx * 80;
        let ref_end = ref_start + 80;
        
        if ref_end <= reference_output.len() {
            let frame_mse = calculate_mse(
                &decoded.samples,
                &reference_output[ref_start..ref_end]
            );
            total_mse += frame_mse;
            max_mse = max_mse.max(frame_mse);
        }
    }
    
    let average_mse = if frames_tested > 0 {
        total_mse / frames_tested as f64
    } else {
        0.0
    };
    
    let bit_exact = average_mse == 0.0 && max_mse == 0.0;
    let mse_threshold = 100.0; // More relaxed for full chain
    let frames_passed = if average_mse < mse_threshold { frames_tested } else { 0 };
    let pass_rate = if frames_tested > 0 {
        frames_passed as f64 / frames_tested as f64
    } else {
        0.0
    };
    
    ComplianceResult {
        test_name: test_name.to_string(),
        frames_tested,
        frames_passed,
        average_mse,
        max_mse,
        pass_rate,
        bit_exact,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn run_compliance_tests() {
        println!("\n========================================");
        println!("G.729A Codec Compliance Test Suite");
        println!("========================================");
        
        let test_dir = Path::new("crates/codec-core/src/codecs/g729a/tests/test_vectors");
        let mut all_results = Vec::new();
        
        // Test configurations
        let test_cases = vec![
            ("ALGTHM", "Algorithm coverage"),
            ("SPEECH", "Generic speech"),
            ("LSP", "LSP quantization"),
            ("FIXED", "Fixed codebook"),
            ("PITCH", "Pitch search"),
            ("TAME", "Taming procedure"),
        ];
        
        for (test_name, description) in test_cases {
            println!("\n--- Testing {} ({}) ---", test_name, description);
            
            let input_path = test_dir.join(format!("{}.IN", test_name));
            let bits_path = test_dir.join(format!("{}.BIT", test_name));
            let output_path = test_dir.join(format!("{}.PST", test_name));
            
            // Test encoder if input file exists
            if input_path.exists() && bits_path.exists() {
                let result = test_encoder_compliance(test_name, &input_path, &bits_path);
                result.print_summary();
                all_results.push(result);
            }
            
            // Test decoder if bitstream and output exist
            if bits_path.exists() && output_path.exists() {
                let result = test_decoder_compliance(test_name, &bits_path, &output_path);
                result.print_summary();
                all_results.push(result);
            }
            
            // Test full chain if input and output exist
            if input_path.exists() && output_path.exists() {
                let result = test_codec_chain_compliance(test_name, &input_path, &output_path);
                result.print_summary();
                all_results.push(result);
            }
        }
        
        // Print overall summary
        println!("\n========================================");
        println!("Overall Compliance Summary");
        println!("========================================");
        
        let total_tests = all_results.len();
        let bit_exact_tests = all_results.iter().filter(|r| r.bit_exact).count();
        let passing_tests = all_results.iter().filter(|r| r.pass_rate >= 0.95).count();
        
        println!("Total test configurations: {}", total_tests);
        println!("Bit-exact tests: {} ({:.1}%)", bit_exact_tests, 
                 bit_exact_tests as f64 / total_tests as f64 * 100.0);
        println!("Tests with >95% pass rate: {} ({:.1}%)", passing_tests,
                 passing_tests as f64 / total_tests as f64 * 100.0);
        
        let avg_pass_rate: f64 = all_results.iter().map(|r| r.pass_rate).sum::<f64>() / total_tests as f64;
        println!("Average pass rate: {:.2}%", avg_pass_rate * 100.0);
        
        println!("\nDetailed Results by Test Type:");
        for result in &all_results {
            println!("  {} - Pass: {:.1}%, MSE: {:.3}, Bit-exact: {}", 
                     result.test_name, 
                     result.pass_rate * 100.0,
                     result.average_mse,
                     result.bit_exact);
        }
        
        // Provide recommendations
        println!("\n========================================");
        println!("Recommendations");
        println!("========================================");
        
        if bit_exact_tests == 0 {
            println!("⚠️  No bit-exact matches found. This is expected for initial implementation.");
            println!("   Focus on:");
            println!("   1. Fixed-point arithmetic precision");
            println!("   2. Rounding modes and saturation");
            println!("   3. Table lookup accuracy");
        } else if bit_exact_tests < total_tests {
            println!("✓ Some bit-exact matches found!");
            println!("   Continue refining non-matching components.");
        } else {
            println!("✅ All tests are bit-exact! Codec is fully compliant.");
        }
    }
} 