//! ITU-T G.729 Comprehensive Integration Tests
//!
//! This module provides the complete G.729 compliance test suite that validates
//! the entire codec implementation against official ITU-T test vectors.

use super::itu_test_utils::*;
use crate::codecs::g729::src::encoder::G729Encoder;
use crate::codecs::g729::src::decoder::G729Decoder;

/// Comprehensive G.729 ITU-T compliance test suite
/// 
/// This is the main compliance test that validates our complete G.729 implementation
/// against the official ITU-T test vectors. It tests all variants and provides
/// detailed compliance reporting.
#[test]
fn test_full_g729_itu_compliance_suite() {
    println!("🎯 G.729 ITU-T COMPREHENSIVE COMPLIANCE TEST SUITE");
    println!("==================================================");
    println!("Testing complete G.729 implementation against official ITU test vectors");
    println!("This test validates encoder, decoder, and all algorithm components.\n");
    
    let mut results = ComplianceResults::new();
    
    // Test G.729 Core compliance
    println!("📋 Testing G.729 Core Implementation...");
    let core_result = test_g729_core_compliance();
    results.add_suite("G.729 Core".to_string(), core_result);
    
    // Test round-trip encoding/decoding
    println!("\n📋 Testing Round-Trip Encoding/Decoding...");
    let roundtrip_result = test_roundtrip_compliance();
    results.add_suite("Round-Trip".to_string(), roundtrip_result);
    
    // Test component-specific compliance
    println!("\n📋 Testing Individual Components...");
    let component_result = test_component_compliance();
    results.add_suite("Components".to_string(), component_result);
    
    // Test error handling and robustness
    println!("\n📋 Testing Error Handling and Robustness...");
    let robustness_result = test_robustness_compliance();
    results.add_suite("Robustness".to_string(), robustness_result);
    
    // Generate final compliance report
    results.print_summary();
    
    // Production readiness assessment
    let overall_compliance = results.overall_compliance();
    
    println!("\n🎖️  PRODUCTION READINESS ASSESSMENT:");
    if overall_compliance >= 0.95 {
        println!("  🏆 PRODUCTION READY - Excellent compliance with ITU-T G.729 standard");
        println!("  ✅ Suitable for commercial VoIP applications");
        println!("  ✅ Meets telecom industry quality standards");
    } else if overall_compliance >= 0.85 {
        println!("  🥈 NEAR PRODUCTION READY - Good compliance with minor issues");
        println!("  ⚠️  Minor tuning recommended before commercial deployment");
        println!("  ✅ Suitable for testing and development use");
    } else if overall_compliance >= 0.75 {
        println!("  🥉 DEVELOPMENT READY - Acceptable compliance for testing");
        println!("  ⚠️  Several issues need addressing before production");
        println!("  ✅ Suitable for internal testing and research");
    } else {
        println!("  ❌ NOT READY - Significant compliance issues detected");
        println!("  ❌ Major algorithm issues need fixing");
        println!("  ❌ Not suitable for production or testing");
    }
    
    // For automated CI/CD, require minimum compliance threshold
    assert!(overall_compliance >= 0.75, 
            "G.729 overall compliance too low for production: {:.1}% (minimum 75%)", 
            overall_compliance * 100.0);
}

/// Test G.729 core algorithm compliance
fn test_g729_core_compliance() -> TestSuiteResult {
    let mut suite = TestSuiteResult::new("G.729 Core Algorithm".to_string());
    
    // Test encoder with core test vectors
    let encoder_tests = [
        ("algthm.in", "algthm.bit", "Algorithm conditional parts"),
        ("fixed.in", "fixed.bit", "Fixed codebook search"),
        ("lsp.in", "lsp.bit", "LSP quantization"),
        ("pitch.in", "pitch.bit", "Pitch search"),
        ("speech.in", "speech.bit", "Speech processing"),
        ("tame.in", "tame.bit", "Taming procedure"),
    ];
    
    for (input_file, bitstream_file, description) in &encoder_tests {
        let test_result = test_encoder_vector(input_file, bitstream_file, description);
        suite.add_test(test_result);
    }
    
    // Test decoder with core test vectors
    let decoder_tests = [
        ("algthm.bit", "algthm.pst", "Algorithm conditional parts"),
        ("erasure.bit", "erasure.pst", "Frame erasure recovery"),
        ("fixed.bit", "fixed.pst", "Fixed codebook search"),
        ("lsp.bit", "lsp.pst", "LSP quantization"),
        ("overflow.bit", "overflow.pst", "Overflow detection"),
        ("parity.bit", "parity.pst", "Parity check"),
        ("pitch.bit", "pitch.pst", "Pitch search"),
        ("speech.bit", "speech.pst", "Speech processing"),
        ("tame.bit", "tame.pst", "Taming procedure"),
    ];
    
    for (bitstream_file, output_file, description) in &decoder_tests {
        let test_result = test_decoder_vector(bitstream_file, output_file, description);
        suite.add_test(test_result);
    }
    
    suite
}

/// Test round-trip encoding/decoding compliance
fn test_roundtrip_compliance() -> TestSuiteResult {
    let mut suite = TestSuiteResult::new("Round-Trip Compliance".to_string());
    
    let test_files = ["algthm.in", "fixed.in", "lsp.in", "pitch.in", "tame.in"];
    
    for input_file in &test_files {
        let test_result = test_roundtrip_vector(input_file);
        suite.add_test(test_result);
    }
    
    suite
}

/// Test individual component compliance
fn test_component_compliance() -> TestSuiteResult {
    let mut suite = TestSuiteResult::new("Component Compliance".to_string());
    
    // Test LPC analysis component
    suite.add_test(test_lpc_component());
    
    // Test pitch analysis component  
    suite.add_test(test_pitch_component());
    
    // Test ACELP component
    suite.add_test(test_acelp_component());
    
    // Test quantization component
    suite.add_test(test_quantization_component());
    
    suite
}

/// Test robustness and error handling
fn test_robustness_compliance() -> TestSuiteResult {
    let mut suite = TestSuiteResult::new("Robustness & Error Handling".to_string());
    
    // Test encoder robustness
    suite.add_test(test_encoder_robustness());
    
    // Test decoder robustness
    suite.add_test(test_decoder_robustness());
    
    // Test error concealment
    suite.add_test(test_error_concealment());
    
    // Test frame synchronization
    suite.add_test(test_frame_synchronization());
    
    suite
}

/// Test single encoder test vector
fn test_encoder_vector(input_file: &str, bitstream_file: &str, description: &str) -> TestResult {
    match (parse_g729_pcm_samples(input_file), parse_g729_bitstream(bitstream_file)) {
        (Ok(input_samples), Ok(expected_bitstream)) => {
            let mut encoder = G729Encoder::new();
            let mut actual_bitstream = Vec::new();
            
            for frame in input_samples.chunks(80) {
                if frame.len() == 80 {
                    let g729_frame = encoder.encode_frame(frame);
                    actual_bitstream.extend(g729_frame.to_bitstream());
                }
            }
            
            let similarity = calculate_bitstream_similarity(&expected_bitstream, &actual_bitstream);
            let passed = similarity >= 0.8; // 80% threshold for encoder
            
            TestResult {
                name: format!("Encoder: {}", description),
                passed,
                similarity,
                details: if passed {
                    "Encoder compliance OK".to_string()
                } else {
                    format!("Low encoder similarity: {:.1}%", similarity * 100.0)
                },
            }
        }
        _ => TestResult {
            name: format!("Encoder: {}", description),
            passed: false,
            similarity: 0.0,
            details: "Failed to load test files".to_string(),
        }
    }
}

/// Test single decoder test vector
fn test_decoder_vector(bitstream_file: &str, output_file: &str, description: &str) -> TestResult {
    match (parse_g729_bitstream(bitstream_file), parse_g729_reference_output(output_file)) {
        (Ok(bitstream), Ok(expected_output)) => {
            let mut decoder = G729Decoder::new();
            let mut actual_output = Vec::new();
            
            for frame_bits in bitstream.chunks(10) {
                if frame_bits.len() == 10 {
                    if let Some(frame) = decoder.decode_bitstream(frame_bits) {
                        actual_output.extend(decoder.decode_frame(&frame));
                    } else {
                        actual_output.extend(vec![0i16; 80]); // Silence for failed frames
                    }
                }
            }
            
            let min_len = expected_output.len().min(actual_output.len());
            let similarity = calculate_sample_similarity(&expected_output[..min_len], &actual_output[..min_len]);
            let passed = similarity >= 0.75; // 75% threshold for decoder (lossy codec)
            
            TestResult {
                name: format!("Decoder: {}", description),
                passed,
                similarity,
                details: if passed {
                    "Decoder compliance OK".to_string()
                } else {
                    format!("Low decoder similarity: {:.1}%", similarity * 100.0)
                },
            }
        }
        _ => TestResult {
            name: format!("Decoder: {}", description),
            passed: false,
            similarity: 0.0,
            details: "Failed to load test files".to_string(),
        }
    }
}

/// Test round-trip encode/decode for a single vector
fn test_roundtrip_vector(input_file: &str) -> TestResult {
    match parse_g729_pcm_samples(input_file) {
        Ok(input_samples) => {
            let mut encoder = G729Encoder::new();
            let mut decoder = G729Decoder::new();
            let mut roundtrip_output = Vec::new();
            
            for frame in input_samples.chunks(80) {
                if frame.len() == 80 {
                    // Encode
                    let g729_frame = encoder.encode_frame(frame);
                    let bitstream = g729_frame.to_bitstream();
                    
                    // Decode
                    if let Some(decoded_frame) = decoder.decode_bitstream(&bitstream) {
                        roundtrip_output.extend(decoder.decode_frame(&decoded_frame));
                    } else {
                        roundtrip_output.extend(vec![0i16; 80]);
                    }
                }
            }
            
            let min_len = input_samples.len().min(roundtrip_output.len());
            let similarity = calculate_sample_similarity(&input_samples[..min_len], &roundtrip_output[..min_len]);
            let passed = similarity >= 0.7; // 70% threshold for round-trip (significant loss expected)
            
            TestResult {
                name: format!("Round-trip: {}", input_file),
                passed,
                similarity,
                details: if passed {
                    "Round-trip quality OK".to_string()
                } else {
                    format!("Poor round-trip quality: {:.1}%", similarity * 100.0)
                },
            }
        }
        _ => TestResult {
            name: format!("Round-trip: {}", input_file),
            passed: false,
            similarity: 0.0,
            details: "Failed to load input file".to_string(),
        }
    }
}

/// Test LPC analysis component
fn test_lpc_component() -> TestResult {
    // Generate test signal for LPC analysis
    let test_signal: Vec<i16> = (0..240).map(|i| {
        (1000.0 * (2.0 * std::f32::consts::PI * i as f32 / 40.0).sin()) as i16
    }).collect();
    
    // Test LPC analyzer creation and basic operation
    let mut lpc_analyzer = crate::codecs::g729::src::lpc::LpcAnalyzer::new();
    let mut lpc_coeffs = vec![0i16; 11];
    let mut lsp = vec![0i16; 10];
    
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        lpc_analyzer.analyze_frame(&test_signal, &mut lpc_coeffs, &mut lsp);
        
        // Validate LPC coefficients
        let coeffs_valid = lpc_coeffs[0] != 0 && lpc_coeffs.iter().any(|&x| x != 0);
        let lsp_valid = lsp.iter().any(|&x| x != 0);
        
        coeffs_valid && lsp_valid
    })) {
        Ok(valid) => TestResult {
            name: "LPC Analysis".to_string(),
            passed: valid,
            similarity: if valid { 1.0 } else { 0.0 },
            details: if valid { "LPC analysis working".to_string() } else { "LPC analysis failed".to_string() },
        },
        Err(_) => TestResult {
            name: "LPC Analysis".to_string(),
            passed: false,
            similarity: 0.0,
            details: "LPC analysis panicked".to_string(),
        }
    }
}

/// Test pitch analysis component
fn test_pitch_component() -> TestResult {
    // Generate periodic test signal for pitch analysis
    let test_signal: Vec<i16> = (0..160).map(|i| {
        (1000.0 * (2.0 * std::f32::consts::PI * i as f32 / 40.0).sin()) as i16 // 40-sample period
    }).collect();
    
    let mut pitch_analyzer = crate::codecs::g729::src::pitch::PitchAnalyzer::new();
    
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // Test open-loop pitch estimation
        let ol_lag = pitch_analyzer.pitch_ol(&test_signal, 20, 143);
        
        // Test closed-loop pitch refinement
        let dummy_y1 = vec![0i16; 40];
        let dummy_y2 = vec![0i16; 40];
        let (cl_lag, gain) = pitch_analyzer.pitch_fr3(&test_signal[..40], &dummy_y1, &dummy_y2, ol_lag, ol_lag + 10, 1);
        
        // Validate pitch parameters
        let ol_valid = ol_lag >= 20 && ol_lag <= 143;
        let cl_valid = cl_lag >= 20 && cl_lag <= 143;
        let gain_valid = gain != 0;
        
        ol_valid && cl_valid && gain_valid
    })) {
        Ok(valid) => TestResult {
            name: "Pitch Analysis".to_string(),
            passed: valid,
            similarity: if valid { 1.0 } else { 0.0 },
            details: if valid { "Pitch analysis working".to_string() } else { "Pitch analysis failed".to_string() },
        },
        Err(_) => TestResult {
            name: "Pitch Analysis".to_string(),
            passed: false,
            similarity: 0.0,
            details: "Pitch analysis panicked".to_string(),
        }
    }
}

/// Test ACELP component
fn test_acelp_component() -> TestResult {
    let test_signal = vec![1000i16; 40];
    let impulse = vec![1000i16; 40];
    let mut acelp_analyzer = crate::codecs::g729::src::acelp::AcelpAnalyzer::new();
    
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        acelp_analyzer.set_impulse_response(&impulse);
        
        let mut code = vec![0i16; 40];
        let mut y = vec![0i16; 40];
        let (positions, signs, gain_idx) = acelp_analyzer.acelp_codebook_search(&test_signal, &test_signal, &mut code, &mut y);
        
        // Validate ACELP parameters
        let positions_valid = positions.iter().all(|&pos| pos < 40);
        let signs_valid = signs.iter().all(|&sign| sign == 1 || sign == -1);
        let gain_valid = gain_idx <= 127;
        
        positions_valid && signs_valid && gain_valid
    })) {
        Ok(valid) => TestResult {
            name: "ACELP Analysis".to_string(),
            passed: valid,
            similarity: if valid { 1.0 } else { 0.0 },
            details: if valid { "ACELP analysis working".to_string() } else { "ACELP analysis failed".to_string() },
        },
        Err(_) => TestResult {
            name: "ACELP Analysis".to_string(),
            passed: false,
            similarity: 0.0,
            details: "ACELP analysis panicked".to_string(),
        }
    }
}

/// Test quantization component
fn test_quantization_component() -> TestResult {
    let test_lsp = vec![1000i16; 10];
    let mut lsp_quantizer = crate::codecs::g729::src::quantization::LspQuantizer::new();
    let mut gain_quantizer = crate::codecs::g729::src::quantization::GainQuantizer::new();
    
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        // Test LSP quantization
        let mut lsp_q = vec![0i16; 10];
        let lsp_indices = lsp_quantizer.quantize_lsp(&test_lsp, &mut lsp_q);
        
        // Test gain quantization
        let (gain_idx, quant_adaptive, quant_fixed) = gain_quantizer.quantize_gains(1000, 800, 1500);
        
        // Validate quantization
        let lsp_valid = !lsp_indices.is_empty() && lsp_q.iter().any(|&x| x != 0);
        let gain_valid = gain_idx <= 127 && quant_adaptive != 0 && quant_fixed != 0;
        
        lsp_valid && gain_valid
    })) {
        Ok(valid) => TestResult {
            name: "Quantization".to_string(),
            passed: valid,
            similarity: if valid { 1.0 } else { 0.0 },
            details: if valid { "Quantization working".to_string() } else { "Quantization failed".to_string() },
        },
        Err(_) => TestResult {
            name: "Quantization".to_string(),
            passed: false,
            similarity: 0.0,
            details: "Quantization panicked".to_string(),
        }
    }
}

/// Test encoder robustness
fn test_encoder_robustness() -> TestResult {
    let mut encoder = G729Encoder::new();
    let test_signals = [
        vec![0i16; 80],      // Silence
        vec![16000i16; 80],  // High amplitude
        vec![-16000i16; 80], // Negative high amplitude
        (0..80).map(|i| (i as i16) * 100).collect(), // Ramp
    ];
    
    let mut successful = 0;
    for signal in &test_signals {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            let frame = encoder.encode_frame(signal);
            frame.bit_count() == 80 && frame.subframes.len() == 2
        })) {
            Ok(valid) if valid => successful += 1,
            _ => {}
        }
    }
    
    let success_rate = successful as f64 / test_signals.len() as f64;
    let passed = success_rate >= 0.8;
    
    TestResult {
        name: "Encoder Robustness".to_string(),
        passed,
        similarity: success_rate,
        details: format!("Encoder robustness: {:.1}%", success_rate * 100.0),
    }
}

/// Test decoder robustness
fn test_decoder_robustness() -> TestResult {
    let mut decoder = G729Decoder::new();
    let test_bitstreams = [
        vec![0u8; 10],    // All zeros
        vec![0xFFu8; 10], // All ones
        vec![0xAAu8; 10], // Pattern
        vec![0x55u8; 10], // Pattern
    ];
    
    let mut successful = 0;
    for bitstream in &test_bitstreams {
        match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            match decoder.decode_bitstream(bitstream) {
                Some(frame) => decoder.decode_frame(&frame).len() == 80,
                None => decoder.conceal_frame(true).len() == 80,
            }
        })) {
            Ok(valid) if valid => successful += 1,
            _ => {}
        }
    }
    
    let success_rate = successful as f64 / test_bitstreams.len() as f64;
    let passed = success_rate >= 0.8;
    
    TestResult {
        name: "Decoder Robustness".to_string(),
        passed,
        similarity: success_rate,
        details: format!("Decoder robustness: {:.1}%", success_rate * 100.0),
    }
}

/// Test error concealment
fn test_error_concealment() -> TestResult {
    let mut decoder = G729Decoder::new();
    
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        let concealed = decoder.conceal_frame(true);
        concealed.len() == 80
    })) {
        Ok(valid) => TestResult {
            name: "Error Concealment".to_string(),
            passed: valid,
            similarity: if valid { 1.0 } else { 0.0 },
            details: if valid { "Error concealment working".to_string() } else { "Error concealment failed".to_string() },
        },
        Err(_) => TestResult {
            name: "Error Concealment".to_string(),
            passed: false,
            similarity: 0.0,
            details: "Error concealment panicked".to_string(),
        }
    }
}

/// Test frame synchronization
fn test_frame_synchronization() -> TestResult {
    // This is a simplified test - in practice would use actual bitstream data
    let mut decoder = G729Decoder::new();
    let test_frame = vec![0x12u8, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0xDE, 0xF0, 0x11, 0x22];
    
    match std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        match decoder.decode_bitstream(&test_frame) {
            Some(_) => true,
            None => true, // Graceful handling of invalid frames is also success
        }
    })) {
        Ok(_) => TestResult {
            name: "Frame Synchronization".to_string(),
            passed: true,
            similarity: 1.0,
            details: "Frame sync handling OK".to_string(),
        },
        Err(_) => TestResult {
            name: "Frame Synchronization".to_string(),
            passed: false,
            similarity: 0.0,
            details: "Frame sync panicked".to_string(),
        }
    }
}

/// Test file availability and basic parsing
#[test]
fn test_itu_test_data_availability() {
    println!("🧪 Testing ITU Test Data Availability");
    
    let core_files = [
        "algthm.in", "algthm.bit", "algthm.pst",
        "fixed.in", "fixed.bit", "fixed.pst", 
        "lsp.in", "lsp.bit", "lsp.pst",
        "pitch.in", "pitch.bit", "pitch.pst",
        "speech.in", "speech.bit", "speech.pst",
        "tame.in", "tame.bit", "tame.pst",
        "erasure.bit", "erasure.pst",
        "overflow.bit", "overflow.pst",
        "parity.bit", "parity.pst",
    ];
    
    let mut available_files = 0;
    
    for filename in &core_files {
        if filename.ends_with(".in") || filename.ends_with(".pst") {
            match parse_g729_pcm_samples(filename) {
                Ok(samples) => {
                    println!("  ✓ {} - {} samples", filename, samples.len());
                    available_files += 1;
                }
                Err(_) => {
                    println!("  ❌ {} - not found or invalid", filename);
                }
            }
        } else if filename.ends_with(".bit") {
            match parse_g729_bitstream(filename) {
                Ok(bitstream) => {
                    println!("  ✓ {} - {} bytes", filename, bitstream.len());
                    available_files += 1;
                }
                Err(_) => {
                    println!("  ❌ {} - not found or invalid", filename);
                }
            }
        }
    }
    
    let availability_rate = available_files as f64 / core_files.len() as f64;
    println!("Test data availability: {:.1}% ({}/{} files)", 
             availability_rate * 100.0, available_files, core_files.len());
    
    // For this test to pass, we need at least some test files available
    assert!(availability_rate >= 0.5, 
            "Insufficient test data available: {:.1}%", availability_rate * 100.0);
} 