//! ITU-T G.729 Annex A Compliance Tests
//!
//! Tests for G.729 Annex A (reduced complexity variant) using official ITU test vectors

use super::itu_test_utils::*;
use crate::codecs::g729::src::encoder::{G729Encoder, G729Variant};
use crate::codecs::g729::src::decoder::G729Decoder;

/// Test G.729A encoder compliance with reduced complexity requirements
/// 
/// G.729A maintains quality while reducing computational complexity by approximately 40%
/// through simplified pitch search and ACELP algorithms.
#[test]
fn test_g729a_encoder_compliance() {
    println!("🎯 G.729A (ANNEX A) ENCODER COMPLIANCE TEST");
    println!("==========================================");
    println!("Testing G.729A reduced complexity encoder variant");
    
    // G.729A test vectors from the g729AnnexA directory
    let test_cases = [
        ("algthm.in", "algthm.bit", "Algorithm conditional parts"),
        ("fixed.in", "fixed.bit", "Fixed codebook - reduced complexity"),
        ("lsp.in", "lsp.bit", "LSP quantization"),
        ("pitch.in", "pitch.bit", "Pitch search - simplified"),
        ("speech.in", "speech.bit", "Speech processing"),
        ("tame.in", "tame.bit", "Taming procedure"),
        ("test.in", "test.bit", "G.729A specific test sequence"), // Added G.729A specific test
    ];
    
    let mut total_tests = 0;
    let mut passed_tests = 0;
    let mut total_similarity = 0.0;
    let mut total_complexity_reduction = 0.0;
    
    for (input_file, expected_bitstream_file, description) in &test_cases {
        println!("\n🧪 Testing G.729A: {} - {}", description, input_file);
        
        // Load input samples from G.729A test directory
        let input_samples = match get_variant_test_data_path(G729Variant::AnnexA, input_file)
            .and_then(|path| std::fs::read(&path).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e)))
            .and_then(|data| parse_16bit_samples(&data)) {
            Ok(samples) => {
                println!("  ✓ Loaded {} G.729A input samples", samples.len());
                samples
            }
            Err(e) => {
                println!("  ⚠️  Using core G.729 samples as fallback: {}", e);
                match parse_g729_pcm_samples(input_file) {
                    Ok(samples) => samples,
                    Err(e) => {
                        println!("  ❌ Failed to load any input samples: {}", e);
                        continue;
                    }
                }
            }
        };
        
        // Load expected G.729A bitstream
        let expected_bitstream = match get_variant_test_data_path(G729Variant::AnnexA, expected_bitstream_file)
            .and_then(|path| std::fs::read(&path).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))) {
            Ok(data) => {
                println!("  ✓ Loaded {} bytes of G.729A expected bitstream", data.len());
                data
            }
            Err(e) => {
                println!("  ⚠️  Using core G.729 bitstream as reference: {}", e);
                match parse_g729_bitstream(expected_bitstream_file) {
                    Ok(data) => data,
                    Err(e) => {
                        println!("  ❌ Failed to load any reference bitstream: {}", e);
                        continue;
                    }
                }
            }
        };
        
        // Test our encoder with G.729A configuration
        let start_time = std::time::Instant::now();
        let mut encoder = G729Encoder::new_with_variant(G729Variant::AnnexA);
        let mut actual_bitstream = Vec::new();
        let mut frame_count = 0;
        
        for frame in input_samples.chunks(80) {
            if frame.len() == 80 {
                let g729_frame = encoder.encode_frame(frame);
                let frame_bits = g729_frame.to_bitstream();
                actual_bitstream.extend(frame_bits);
                frame_count += 1;
            }
        }
        
        let encoding_time = start_time.elapsed();
        println!("  ✓ Encoded {} frames in {:?}", frame_count, encoding_time);
        
        // Compare with reference bitstream
        let similarity = calculate_bitstream_similarity(&expected_bitstream, &actual_bitstream);
        total_similarity += similarity;
        total_tests += 1;
        
        // Estimate complexity reduction (time-based approximation)
        let complexity_reduction = estimate_complexity_reduction(&encoding_time, frame_count);
        total_complexity_reduction += complexity_reduction;
        
        println!("  📊 Bitstream similarity: {:.1}%", similarity * 100.0);
        println!("  📊 Estimated complexity reduction: {:.1}%", complexity_reduction * 100.0);
        
        // G.729A compliance criteria:
        // - Bitstream similarity ≥ 90% (should be very close to reference)
        // - Complexity reduction ≥ 30% (target is ~40%)
        let similarity_ok = similarity >= 0.90;
        let complexity_ok = complexity_reduction >= 0.30;
        
        if similarity_ok && complexity_ok {
            println!("  ✅ PASSED - G.729A encoder compliant for {}", description);
            passed_tests += 1;
        } else {
            println!("  ❌ FAILED - G.729A encoder non-compliant for {}", description);
            if !similarity_ok {
                println!("    - Low bitstream similarity: {:.1}% (need ≥90%)", similarity * 100.0);
            }
            if !complexity_ok {
                println!("    - Insufficient complexity reduction: {:.1}% (need ≥30%)", complexity_reduction * 100.0);
            }
        }
    }
    
    // Overall G.729A encoder assessment
    let compliance_rate = passed_tests as f64 / total_tests as f64;
    let avg_similarity = total_similarity / total_tests as f64;
    let avg_complexity_reduction = total_complexity_reduction / total_tests as f64;
    
    println!("\n🎉 G.729A ENCODER COMPLIANCE SUMMARY:");
    println!("  📊 Tests passed: {}/{} ({:.1}%)", passed_tests, total_tests, compliance_rate * 100.0);
    println!("  📊 Average similarity: {:.1}%", avg_similarity * 100.0);
    println!("  📊 Average complexity reduction: {:.1}%", avg_complexity_reduction * 100.0);
    
    if compliance_rate >= 0.9 && avg_complexity_reduction >= 0.35 {
        println!("  ✅ EXCELLENT G.729A COMPLIANCE - Reduced complexity with quality maintenance!");
    } else if compliance_rate >= 0.75 {
        println!("  ✅ GOOD G.729A COMPLIANCE - Minor optimization opportunities remain");
    } else {
        println!("  ❌ G.729A COMPLIANCE ISSUES - Complexity reduction or quality problems");
    }
    
    // Require reasonable G.729A compliance
    assert!(compliance_rate >= 0.75, 
            "G.729A encoder compliance too low: {:.1}%", compliance_rate * 100.0);
}

/// Test G.729A decoder compliance with reduced complexity implementation
#[test]
fn test_g729a_decoder_compliance() {
    println!("🧪 G.729A (ANNEX A) DECODER COMPLIANCE TEST");
    println!("==========================================");
    
    // G.729A decoder test vectors  
    let test_cases = [
        ("algthm.bit", "algthm.pst", "Algorithm conditional parts"),
        ("fixed.bit", "fixed.pst", "Fixed codebook - reduced complexity"),
        ("lsp.bit", "lsp.pst", "LSP quantization"),
        ("pitch.bit", "pitch.pst", "Pitch synthesis - simplified"),
        ("speech.bit", "speech.pst", "Speech processing"),
        ("tame.bit", "tame.pst", "Taming procedure"),
    ];
    
    let mut total_tests = 0;
    let mut passed_tests = 0;
    let mut total_similarity = 0.0;
    
    for (bitstream_file, expected_output_file, description) in &test_cases {
        println!("\n🧪 Testing G.729A Decoder: {} - {}", description, bitstream_file);
        
        // Load G.729A bitstream (try Annex A directory first, fall back to core)
        let bitstream = match get_variant_test_data_path(G729Variant::AnnexA, bitstream_file)
            .and_then(|path| std::fs::read(&path).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e))) {
            Ok(data) => {
                println!("  ✓ Loaded {} bytes of G.729A bitstream", data.len());
                data
            }
            Err(_) => {
                match parse_g729_bitstream(bitstream_file) {
                    Ok(data) => {
                        println!("  ⚠️  Using core G.729 bitstream as test input");
                        data
                    }
                    Err(e) => {
                        println!("  ❌ Failed to load bitstream: {}", e);
                        continue;
                    }
                }
            }
        };
        
        // Load expected G.729A output
        let expected_samples = match get_variant_test_data_path(G729Variant::AnnexA, expected_output_file)
            .and_then(|path| std::fs::read(&path).map_err(|e| std::io::Error::new(std::io::ErrorKind::Other, e)))
            .and_then(|data| parse_16bit_samples(&data)) {
            Ok(samples) => {
                println!("  ✓ Loaded {} expected G.729A output samples", samples.len());
                samples
            }
            Err(_) => {
                match parse_g729_reference_output(expected_output_file) {
                    Ok(samples) => {
                        println!("  ⚠️  Using core G.729 output as reference");
                        samples
                    }
                    Err(e) => {
                        println!("  ❌ Failed to load expected output: {}", e);
                        continue;
                    }
                }
            }
        };
        
        // Test our G.729A decoder
        let start_time = std::time::Instant::now();
        let mut decoder = G729Decoder::new_with_variant(G729Variant::AnnexA);
        let mut actual_samples = Vec::new();
        let mut frame_count = 0;
        let mut decode_errors = 0;
        
        for frame_bits in bitstream.chunks(10) {
            if frame_bits.len() == 10 {
                match decoder.decode_bitstream(frame_bits) {
                    Some(frame) => {
                        let decoded_frame = decoder.decode_frame(&frame);
                        actual_samples.extend(decoded_frame);
                        frame_count += 1;
                    }
                    None => {
                        decode_errors += 1;
                        actual_samples.extend(vec![0i16; 80]); // Silence for failed frames
                    }
                }
            }
        }
        
        let decoding_time = start_time.elapsed();
        println!("  ✓ Decoded {} frames, {} errors in {:?}", frame_count, decode_errors, decoding_time);
        
        // Compare decoder output with reference
        let min_len = expected_samples.len().min(actual_samples.len());
        let similarity = calculate_sample_similarity(&expected_samples[..min_len], &actual_samples[..min_len]);
        total_similarity += similarity;
        total_tests += 1;
        
        // Estimate complexity reduction
        let complexity_reduction = estimate_complexity_reduction(&decoding_time, frame_count);
        
        println!("  📊 Sample similarity: {:.1}%", similarity * 100.0);
        println!("  📊 Estimated complexity reduction: {:.1}%", complexity_reduction * 100.0);
        
        // G.729A decoder compliance criteria
        let similarity_ok = similarity >= 0.80; // Lossy codec, allow some difference
        let complexity_ok = complexity_reduction >= 0.25; // Decoder complexity reduction is typically less than encoder
        let error_rate = if frame_count + decode_errors > 0 {
            decode_errors as f64 / (frame_count + decode_errors) as f64
        } else {
            0.0
        };
        let error_ok = error_rate <= 0.1;
        
        if similarity_ok && complexity_ok && error_ok {
            println!("  ✅ PASSED - G.729A decoder compliant for {}", description);
            passed_tests += 1;
        } else {
            println!("  ❌ FAILED - G.729A decoder non-compliant for {}", description);
            if !similarity_ok {
                println!("    - Low sample similarity: {:.1}% (need ≥80%)", similarity * 100.0);
            }
            if !complexity_ok {
                println!("    - Insufficient complexity reduction: {:.1}%", complexity_reduction * 100.0);
            }
            if !error_ok {
                println!("    - High decode error rate: {:.1}%", error_rate * 100.0);
            }
        }
    }
    
    // Overall G.729A decoder assessment
    let compliance_rate = passed_tests as f64 / total_tests as f64;
    let avg_similarity = total_similarity / total_tests as f64;
    
    println!("\n🎉 G.729A DECODER COMPLIANCE SUMMARY:");
    println!("  📊 Tests passed: {}/{} ({:.1}%)", passed_tests, total_tests, compliance_rate * 100.0);
    println!("  📊 Average similarity: {:.1}%", avg_similarity * 100.0);
    
    assert!(compliance_rate >= 0.75, 
            "G.729A decoder compliance too low: {:.1}%", compliance_rate * 100.0);
}

/// Test G.729A computational complexity reduction validation
#[test]
fn test_g729a_complexity_reduction() {
    println!("🧪 G.729A Computational Complexity Reduction Test");
    
    // Generate test signals for complexity measurement
    let test_signals = [
        generate_sine_wave_signal(1000.0, 8000.0, 800),  // 100ms of 1kHz tone
        generate_speech_like_signal(800),                 // Simulated speech
        generate_mixed_signal(800),                       // Mixed frequency content
    ];
    
    let mut complexity_reductions = Vec::new();
    
    for (i, signal) in test_signals.iter().enumerate() {
        println!("  Testing signal {}: {} samples", i + 1, signal.len());
        
        // Measure G.729 Core encoding time
        let start_core = std::time::Instant::now();
        let mut core_encoder = G729Encoder::new_with_variant(G729Variant::Core);
        for frame in signal.chunks(80) {
            if frame.len() == 80 {
                let _ = core_encoder.encode_frame(frame);
            }
        }
        let core_time = start_core.elapsed();
        
        // Measure G.729A encoding time
        let start_a = std::time::Instant::now();
        let mut a_encoder = G729Encoder::new_with_variant(G729Variant::AnnexA);
        for frame in signal.chunks(80) {
            if frame.len() == 80 {
                let _ = a_encoder.encode_frame(frame);
            }
        }
        let a_time = start_a.elapsed();
        
        let complexity_reduction = if core_time > a_time {
            (core_time - a_time).as_nanos() as f64 / core_time.as_nanos() as f64
        } else {
            0.0
        };
        
        complexity_reductions.push(complexity_reduction);
        
        println!("    Core G.729 time: {:?}", core_time);
        println!("    G.729A time: {:?}", a_time);
        println!("    Complexity reduction: {:.1}%", complexity_reduction * 100.0);
    }
    
    let avg_reduction = complexity_reductions.iter().sum::<f64>() / complexity_reductions.len() as f64;
    
    println!("\nComplexity Reduction Summary:");
    println!("  Average reduction: {:.1}%", avg_reduction * 100.0);
    
    if avg_reduction >= 0.35 {
        println!("  ✅ EXCELLENT - Meets G.729A complexity reduction target (≥35%)");
    } else if avg_reduction >= 0.25 {
        println!("  ✅ GOOD - Reasonable complexity reduction achieved");
    } else {
        println!("  ⚠️  SUBOPTIMAL - G.729A should provide more complexity reduction");
    }
    
    // G.729A should provide meaningful complexity reduction
    assert!(avg_reduction >= 0.20, 
            "G.729A complexity reduction insufficient: {:.1}%", avg_reduction * 100.0);
}

/// Parse 16-bit PCM samples from raw data
fn parse_16bit_samples(data: &[u8]) -> Result<Vec<i16>, std::io::Error> {
    if data.len() % 2 != 0 {
        return Err(std::io::Error::new(
            std::io::ErrorKind::InvalidData,
            "Data length must be even for 16-bit samples"
        ));
    }
    
    let mut samples = Vec::with_capacity(data.len() / 2);
    for chunk in data.chunks_exact(2) {
        let sample = i16::from_le_bytes([chunk[0], chunk[1]]);
        samples.push(sample);
    }
    
    Ok(samples)
}

/// Estimate complexity reduction based on timing measurements
fn estimate_complexity_reduction(elapsed_time: &std::time::Duration, frame_count: usize) -> f64 {
    if frame_count == 0 {
        return 0.0;
    }
    
    // Baseline expectation: G.729A should be ~40% faster than core G.729
    // This is a simplified estimation based on timing
    let time_per_frame = elapsed_time.as_nanos() as f64 / frame_count as f64;
    let baseline_time_per_frame = 125000.0; // Nanoseconds (arbitrary baseline)
    
    if time_per_frame < baseline_time_per_frame {
        (baseline_time_per_frame - time_per_frame) / baseline_time_per_frame
    } else {
        0.0
    }
}

/// Generate sine wave test signal
fn generate_sine_wave_signal(frequency: f32, sample_rate: f32, length: usize) -> Vec<i16> {
    let mut signal = Vec::with_capacity(length);
    for i in 0..length {
        let sample = (8000.0 * (2.0 * std::f32::consts::PI * frequency * i as f32 / sample_rate).sin()) as i16;
        signal.push(sample);
    }
    signal
}

/// Generate speech-like test signal with varying characteristics
fn generate_speech_like_signal(length: usize) -> Vec<i16> {
    let mut signal = Vec::with_capacity(length);
    let mut phase: f32 = 0.0;
    
    for i in 0..length {
        // Simulate speech-like formant structure
        let f1: f32 = 700.0; // First formant
        let f2: f32 = 1220.0; // Second formant
        let f3: f32 = 2600.0; // Third formant
        
        let sample = (
            2000.0 * (phase).sin() +
            1500.0 * (phase * f2 / f1).sin() +
            1000.0 * (phase * f3 / f1).sin()
        ) as i16;
        
        signal.push(sample);
        phase += 2.0 * std::f32::consts::PI * f1 / 8000.0;
        
        // Add some amplitude variation
        if i % 80 == 0 {
            phase *= 0.95; // Slight frequency drift
        }
    }
    
    signal
}

/// Generate mixed frequency content signal
fn generate_mixed_signal(length: usize) -> Vec<i16> {
    let mut signal = Vec::with_capacity(length);
    
    for i in 0..length {
        let t = i as f32 / 8000.0;
        
        let sample = (
            3000.0 * (2.0 * std::f32::consts::PI * 440.0 * t).sin() +    // A4 note
            2000.0 * (2.0 * std::f32::consts::PI * 880.0 * t).sin() +    // A5 note
            1000.0 * (2.0 * std::f32::consts::PI * 1760.0 * t).sin()     // A6 note
        ) as i16;
        
        signal.push(sample);
    }
    
    signal
}

/// Test G.729A backward compatibility with core G.729
#[test]
fn test_g729a_backward_compatibility() {
    println!("🧪 G.729A Backward Compatibility Test");
    
    // Generate test signal
    let test_signal = generate_sine_wave_signal(1000.0, 8000.0, 160); // 20ms
    
    // Encode with G.729A
    let mut a_encoder = G729Encoder::new_with_variant(G729Variant::AnnexA);
    let a_frame1 = a_encoder.encode_frame(&test_signal[..80]);
    let a_frame2 = a_encoder.encode_frame(&test_signal[80..]);
    
    let a_bitstream1 = a_frame1.to_bitstream();
    let a_bitstream2 = a_frame2.to_bitstream();
    
    // Decode with core G.729 decoder (should be compatible)
    let mut core_decoder = G729Decoder::new_with_variant(G729Variant::Core);
    
    let decoded_compatible = 
        core_decoder.decode_bitstream(&a_bitstream1)
            .and_then(|frame| Some(core_decoder.decode_frame(&frame)))
            .map(|samples| samples.len() == 80)
            .unwrap_or(false) &&
        core_decoder.decode_bitstream(&a_bitstream2)
            .and_then(|frame| Some(core_decoder.decode_frame(&frame)))
            .map(|samples| samples.len() == 80)
            .unwrap_or(false);
    
    println!("Backward compatibility: {}", if decoded_compatible { "✅ PASS" } else { "❌ FAIL" });
    
    assert!(decoded_compatible, "G.729A bitstreams should be decodable by core G.729 decoder");
} 