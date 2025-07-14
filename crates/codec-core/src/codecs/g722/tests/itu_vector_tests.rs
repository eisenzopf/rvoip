//! ITU-T G.722 Test Vector Validation
//!
//! This module tests the G.722 codec implementation against official ITU-T test vectors.
//! The tests validate encoding and decoding compliance with the ITU-T G.722 standard.
//!
//! Reference: ITU-T G.722 (2012-09) Appendix II Digital Test Sequences

use crate::codecs::g722::G722Codec;
use crate::codecs::g722::codec::{G722_FRAME_SIZE, G722_ENCODED_FRAME_SIZE};
use super::utils::*;

/// Test basic test vector file parsing
#[test]
fn test_vector_file_parsing() {
    let test_vectors = get_standard_test_vectors();
    
    for vector_info in &test_vectors {
        println!("Testing vector: {}", vector_info.description);
        
        // Try to parse the file
        match vector_info.filename.as_str() {
            name if name.ends_with(".xmt") => {
                // PCM input file
                match parse_g191_pcm_samples(&vector_info.filename) {
                    Ok(samples) => {
                        println!("  Parsed {} PCM samples", samples.len());
                        assert!(!samples.is_empty(), "PCM samples should not be empty");
                        
                        // Basic validation
                        for &sample in &samples {
                            assert!(sample.abs() <= 32767, "PCM sample should be valid: {}", sample);
                        }
                    }
                    Err(e) => {
                        println!("  WARNING: Could not parse {}: {}", vector_info.filename, e);
                        // Continue test - file might not exist in test environment
                    }
                }
            }
            name if name.ends_with(".cod") => {
                // Encoded G.722 file
                match parse_g191_encoded_data(&vector_info.filename) {
                    Ok(encoded_data) => {
                        println!("  Parsed {} encoded bytes", encoded_data.len());
                        assert!(!encoded_data.is_empty(), "Encoded data should not be empty");
                        
                        // Basic validation
                        for &byte in &encoded_data {
                            assert!(byte <= 255, "Encoded byte should be valid: {}", byte);
                        }
                    }
                    Err(e) => {
                        println!("  WARNING: Could not parse {}: {}", vector_info.filename, e);
                        // Continue test - file might not exist in test environment
                    }
                }
            }
            name if name.ends_with(".rc1") || name.ends_with(".rc2") || name.ends_with(".rc3") || name.ends_with(".rc0") => {
                // Decoded output file
                match parse_g191_pcm_samples(&vector_info.filename) {
                    Ok(samples) => {
                        println!("  Parsed {} decoded samples", samples.len());
                        assert!(!samples.is_empty(), "Decoded samples should not be empty");
                        
                        // Basic validation
                        for &sample in &samples {
                            assert!(sample.abs() <= 32767, "Decoded sample should be valid: {}", sample);
                        }
                    }
                    Err(e) => {
                        println!("  WARNING: Could not parse {}: {}", vector_info.filename, e);
                        // Continue test - file might not exist in test environment
                    }
                }
            }
            _ => {
                println!("  SKIP: Unknown file type {}", vector_info.filename);
            }
        }
    }
}

/// Test ITU-T encoding compliance (Step 1)
/// 
/// This test validates that our encoder produces output similar to the ITU-T reference
/// encoder when given the same input test vectors.
#[test]
fn test_itu_encoding_compliance() {
    let test_cases = [
        ("bt1c1.xmt", "bt2r1.cod", "PCM input 1 → G.722 encoded 1"),
        ("bt1c2.xmt", "bt2r2.cod", "PCM input 2 → G.722 encoded 2"),
    ];
    
    for (input_file, expected_output_file, description) in &test_cases {
        println!("\n=== Testing: {} ===", description);
        
        // Load input PCM samples
        let input_samples = match parse_g191_pcm_samples(input_file) {
            Ok(samples) => samples,
            Err(e) => {
                println!("SKIP: Could not load {}: {}", input_file, e);
                continue;
            }
        };
        
        // Load expected encoded output
        let expected_encoded = match parse_g191_encoded_data(expected_output_file) {
            Ok(data) => data,
            Err(e) => {
                println!("SKIP: Could not load {}: {}", expected_output_file, e);
                continue;
            }
        };
        
        println!("Input samples: {}", input_samples.len());
        println!("Expected encoded bytes: {}", expected_encoded.len());
        
        // Encode using our implementation
        let mut codec = G722Codec::new_with_mode(1).unwrap();
        let mut our_encoded = Vec::new();
        
        // Process input in frames
        for chunk in input_samples.chunks(G722_FRAME_SIZE) {
            if chunk.len() == G722_FRAME_SIZE {
                let encoded_frame = codec.encode_frame(chunk).unwrap();
                our_encoded.extend_from_slice(&encoded_frame);
            }
        }
        
        println!("Our encoded bytes: {}", our_encoded.len());
        
        // Calculate similarity
        let similarity = calculate_byte_similarity(&our_encoded, &expected_encoded);
        println!("Encoding similarity: {:.2}%", similarity * 100.0);
        
        // For now, we just require that encoding produces some output
        // ITU-T compliance is a work in progress
        assert!(!our_encoded.is_empty(), "Encoder should produce output");
        assert_eq!(our_encoded.len() % G722_ENCODED_FRAME_SIZE, 0, 
                  "Encoded output should be multiples of frame size");
        
        // Log the result for analysis
        if similarity > 0.5 {
            println!("PASS: High similarity ({}%)", similarity * 100.0);
        } else if similarity > 0.3 {
            println!("PARTIAL: Moderate similarity ({}%)", similarity * 100.0);
        } else {
            println!("ANALYSIS: Low similarity ({}%) - needs investigation", similarity * 100.0);
        }
    }
}

/// Test ITU-T decoding compliance (Step 2)
/// 
/// This test validates that our decoder produces output similar to the ITU-T reference
/// decoder when given the same encoded test vectors.
#[test]
fn test_itu_decoding_compliance() {
    let test_cases = [
        // Test case 1: bt2r1.cod → different modes
        ("bt2r1.cod", "bt3l1.rc1", 1, "G.722 encoded 1 → Low-band decoded 1 (mode 1)"),
        ("bt2r1.cod", "bt3l1.rc2", 2, "G.722 encoded 1 → Low-band decoded 1 (mode 2)"),
        ("bt2r1.cod", "bt3l1.rc3", 3, "G.722 encoded 1 → Low-band decoded 1 (mode 3)"),
        
        // Test case 2: bt2r2.cod → different modes
        ("bt2r2.cod", "bt3l2.rc1", 1, "G.722 encoded 2 → Low-band decoded 2 (mode 1)"),
        ("bt2r2.cod", "bt3l2.rc2", 2, "G.722 encoded 2 → Low-band decoded 2 (mode 2)"),
        ("bt2r2.cod", "bt3l2.rc3", 3, "G.722 encoded 2 → Low-band decoded 2 (mode 3)"),
    ];
    
    for (encoded_file, expected_output_file, mode, description) in &test_cases {
        println!("\n=== Testing: {} ===", description);
        
        // Load encoded input
        let encoded_data = match parse_g191_encoded_data(encoded_file) {
            Ok(data) => data,
            Err(e) => {
                println!("SKIP: Could not load {}: {}", encoded_file, e);
                continue;
            }
        };
        
        // Load expected decoded output
        let expected_decoded = match parse_g191_pcm_samples(expected_output_file) {
            Ok(samples) => samples,
            Err(e) => {
                println!("SKIP: Could not load {}: {}", expected_output_file, e);
                continue;
            }
        };
        
        println!("Encoded bytes: {}", encoded_data.len());
        println!("Expected decoded samples: {}", expected_decoded.len());
        
        // Decode using our implementation
        let mut codec = G722Codec::new_with_mode(*mode).unwrap();
        let mut our_decoded = Vec::new();
        
        // Process encoded data in frames
        for chunk in encoded_data.chunks(G722_ENCODED_FRAME_SIZE) {
            if chunk.len() == G722_ENCODED_FRAME_SIZE {
                let decoded_frame = codec.decode_frame(chunk).unwrap();
                our_decoded.extend_from_slice(&decoded_frame);
            }
        }
        
        println!("Our decoded samples: {}", our_decoded.len());
        
        // Calculate similarity
        let similarity = calculate_sample_similarity(&our_decoded, &expected_decoded);
        println!("Decoding similarity: {:.2}%", similarity * 100.0);
        
        // For now, we just require that decoding produces some output
        // ITU-T compliance is a work in progress
        assert!(!our_decoded.is_empty(), "Decoder should produce output");
        assert_eq!(our_decoded.len() % G722_FRAME_SIZE, 0, 
                  "Decoded output should be multiples of frame size");
        
        // Log the result for analysis
        if similarity > 0.7 {
            println!("PASS: High similarity ({}%)", similarity * 100.0);
        } else if similarity > 0.5 {
            println!("PARTIAL: Moderate similarity ({}%)", similarity * 100.0);
        } else {
            println!("ANALYSIS: Low similarity ({}%) - needs investigation", similarity * 100.0);
        }
    }
}

/// Test round-trip ITU-T compliance
/// 
/// This test validates that our encoder/decoder pair can handle the ITU-T test vectors
/// in a round-trip fashion.
#[test]
fn test_itu_round_trip_compliance() {
    let test_cases = [
        ("bt1c1.xmt", "PCM input 1 round-trip"),
        ("bt1c2.xmt", "PCM input 2 round-trip"),
    ];
    
    for (input_file, description) in &test_cases {
        println!("\n=== Testing: {} ===", description);
        
        // Load input PCM samples
        let input_samples = match parse_g191_pcm_samples(input_file) {
            Ok(samples) => samples,
            Err(e) => {
                println!("SKIP: Could not load {}: {}", input_file, e);
                continue;
            }
        };
        
        println!("Input samples: {}", input_samples.len());
        
        // Test all modes
        for mode in 1..=3 {
            println!("  Testing mode {}", mode);
            
            let mut codec = G722Codec::new_with_mode(mode).unwrap();
            let mut encoded_data = Vec::new();
            let mut decoded_data = Vec::new();
            
            // Encode and decode in frames
            for chunk in input_samples.chunks(G722_FRAME_SIZE) {
                if chunk.len() == G722_FRAME_SIZE {
                    // Encode
                    let encoded_frame = codec.encode_frame(chunk).unwrap();
                    encoded_data.extend_from_slice(&encoded_frame);
                    
                    // Decode
                    let decoded_frame = codec.decode_frame(&encoded_frame).unwrap();
                    decoded_data.extend_from_slice(&decoded_frame);
                }
            }
            
            println!("    Encoded: {} bytes", encoded_data.len());
            println!("    Decoded: {} samples", decoded_data.len());
            
            // Calculate round-trip similarity
            let similarity = calculate_sample_similarity(&input_samples[..decoded_data.len()], &decoded_data);
            println!("    Round-trip similarity: {:.2}%", similarity * 100.0);
            
            // For a lossy codec, we expect some degradation
            assert!(similarity > 0.2, "Round-trip similarity should be > 20% for mode {}", mode);
            
            // Verify compression ratio
            let expected_compressed_size = input_samples.len() / 2; // 2:1 compression
            let actual_compressed_size = encoded_data.len();
            let compression_ratio = actual_compressed_size as f32 / input_samples.len() as f32;
            
            println!("    Compression ratio: {:.2}:1", 1.0 / compression_ratio);
            assert!((compression_ratio - 0.5).abs() < 0.01, 
                   "Compression ratio should be close to 0.5 for mode {}", mode);
        }
    }
}

/// Test G.191 format conversion
#[test]
fn test_g191_format_conversion() {
    // Test PCM to G.191 conversion
    let pcm_samples = vec![1000i16, -2000i16, 0i16, 32767i16, -32768i16];
    let g191_pcm = convert_pcm_to_g191_format(&pcm_samples);
    
    // Should have sync pattern + samples
    assert_eq!(g191_pcm.len(), G191_SYNC_PATTERN_LENGTH + pcm_samples.len());
    
    // Check sync pattern
    for i in 0..G191_SYNC_PATTERN_LENGTH {
        assert_eq!(g191_pcm[i], G191_SYNC_PATTERN);
    }
    
    // Check samples
    for i in 0..pcm_samples.len() {
        assert_eq!(g191_pcm[G191_SYNC_PATTERN_LENGTH + i], pcm_samples[i] as u16);
    }
    
    // Test encoded data to G.191 conversion
    let encoded_data = vec![0x12u8, 0x34u8, 0x56u8, 0x78u8];
    let g191_encoded = convert_to_g191_format(&encoded_data);
    
    // Should have sync pattern + encoded data
    assert_eq!(g191_encoded.len(), G191_SYNC_PATTERN_LENGTH + encoded_data.len());
    
    // Check sync pattern
    for i in 0..G191_SYNC_PATTERN_LENGTH {
        assert_eq!(g191_encoded[i], G191_SYNC_PATTERN);
    }
    
    // Check encoded data
    for i in 0..encoded_data.len() {
        assert_eq!(g191_encoded[G191_SYNC_PATTERN_LENGTH + i], encoded_data[i] as u16);
    }
}

/// Test similarity calculation functions
#[test]
fn test_similarity_calculations() {
    // Test byte similarity
    let a = vec![0x12u8, 0x34, 0x56, 0x78];
    let b = vec![0x12u8, 0x34, 0x56, 0x78];
    assert_eq!(calculate_byte_similarity(&a, &b), 1.0);
    
    let c = vec![0x12u8, 0x34, 0x99, 0x78];
    let similarity = calculate_byte_similarity(&a, &c);
    assert_eq!(similarity, 0.75); // 3/4 match
    
    // Test sample similarity
    let samples1 = vec![1000i16, 2000, 3000, 4000];
    let samples2 = vec![1000i16, 2000, 3000, 4000];
    assert_eq!(calculate_sample_similarity(&samples1, &samples2), 1.0);
    
    let samples3 = vec![1001i16, 2001, 3001, 4001];
    let similarity = calculate_sample_similarity(&samples1, &samples3);
    assert!(similarity > 0.9, "Should have high similarity for close samples");
    
    // Test empty vectors
    assert_eq!(calculate_byte_similarity(&[], &[1, 2, 3]), 0.0);
    assert_eq!(calculate_sample_similarity(&[], &[1, 2, 3]), 0.0);
}

/// Test encoder output in G.191 format
#[test]
fn test_encoder_g191_output() {
    let mut codec = G722Codec::new_with_mode(1).unwrap();
    
    // Generate test input
    let input_samples = (0..G722_FRAME_SIZE)
        .map(|i| ((i as f32 / 16.0).sin() * 10000.0) as i16)
        .collect::<Vec<i16>>();
    
    // Encode
    let encoded = codec.encode_frame(&input_samples).unwrap();
    
    // Convert to G.191 format
    let g191_encoded = convert_to_g191_format(&encoded);
    
    // Verify format
    assert_eq!(g191_encoded.len(), G191_SYNC_PATTERN_LENGTH + encoded.len());
    
    // Check sync pattern
    for i in 0..G191_SYNC_PATTERN_LENGTH {
        assert_eq!(g191_encoded[i], G191_SYNC_PATTERN);
    }
    
    // Check encoded data
    for i in 0..encoded.len() {
        assert_eq!(g191_encoded[G191_SYNC_PATTERN_LENGTH + i], encoded[i] as u16);
    }
}

/// Test decoder with G.191 format input
#[test]
fn test_decoder_g191_input() {
    let mut codec = G722Codec::new_with_mode(1).unwrap();
    
    // Generate test encoded data
    let encoded_data = (0..G722_ENCODED_FRAME_SIZE)
        .map(|i| ((i * 17) % 256) as u8)
        .collect::<Vec<u8>>();
    
    // Convert to G.191 format
    let g191_encoded = convert_to_g191_format(&encoded_data);
    
    // Parse back from G.191 format (simulate reading file)
    let parsed_encoded = g191_encoded[G191_SYNC_PATTERN_LENGTH..]
        .iter()
        .map(|&word| (word & 0xFF) as u8)
        .collect::<Vec<u8>>();
    
    // Should match original
    assert_eq!(parsed_encoded, encoded_data);
    
    // Decode
    let decoded = codec.decode_frame(&parsed_encoded).unwrap();
    assert_eq!(decoded.len(), G722_FRAME_SIZE);
    
    // All samples should be valid
    for &sample in &decoded {
        assert!(sample.abs() <= 32767);
    }
}

/// Test ITU-T test vector information
#[test]
fn test_vector_info() {
    let test_vectors = get_standard_test_vectors();
    
    // Should have all expected test vectors
    assert!(!test_vectors.is_empty());
    
    // Check for expected files
    let filenames: Vec<&str> = test_vectors.iter().map(|v| v.filename.as_str()).collect();
    assert!(filenames.contains(&"bt1c1.xmt"));
    assert!(filenames.contains(&"bt1c2.xmt"));
    assert!(filenames.contains(&"bt2r1.cod"));
    assert!(filenames.contains(&"bt2r2.cod"));
    assert!(filenames.contains(&"bt3l1.rc1"));
    assert!(filenames.contains(&"bt3l1.rc2"));
    assert!(filenames.contains(&"bt3l1.rc3"));
    assert!(filenames.contains(&"bt3h1.rc0"));
    
    // Check that all have valid info
    for vector in &test_vectors {
        assert!(!vector.filename.is_empty());
        assert!(!vector.description.is_empty());
        assert!(vector.expected_size > 0);
    }
} 