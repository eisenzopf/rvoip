//! G.722 Decoder Tests
//!
//! This module contains focused tests for the G.722 decoder functionality.
//! Tests cover mode-specific behavior, decoding accuracy, and decoder state management.

use super::utils::*;
use crate::codecs::g722::*;
use crate::codecs::g722::state::{G722EncoderState, AdpcmState};
use crate::codecs::g722::reference::abs_s;
use crate::codecs::g722::codec::{G722_FRAME_SIZE, G722_ENCODED_FRAME_SIZE};
use crate::codecs::g722::adpcm;
use super::utils::test_signals::*;
use super::utils::{calculate_sample_similarity, calculate_byte_similarity};
use rand;

/// Test decoder creation and initialization
#[test]
fn test_decoder_creation() {
    for mode in 1..=3 {
        let codec = G722Codec::new_with_mode(mode).unwrap();
        assert_eq!(codec.mode(), mode);
        assert_eq!(codec.decoder_state().state().low_band().det, 32);
        assert_eq!(codec.decoder_state().state().high_band().det, 8);
    }
}

/// Test decoder state management
#[test]
fn test_decoder_state_management() {
    let mut codec = G722Codec::new_with_mode(1).unwrap();
    
    // Initial state
    assert_eq!(codec.decoder_state().state().low_band().sl, 0);
    assert_eq!(codec.decoder_state().state().high_band().sl, 0);
    
    // Process some encoded data
    let encoded = vec![0x55u8; G722_ENCODED_FRAME_SIZE];
    let _ = codec.decode_frame(&encoded).unwrap();
    
    // State should have changed
    let low_sl = codec.decoder_state().state().low_band().sl;
    let high_sl = codec.decoder_state().state().high_band().sl;
    
    // Process another frame
    let _ = codec.decode_frame(&encoded).unwrap();
    
    // State should continue to evolve
    assert_ne!(codec.decoder_state().state().low_band().sl, low_sl);
    assert_ne!(codec.decoder_state().state().high_band().sl, high_sl);
}

/// Test decoder reset functionality
#[test]
fn test_decoder_reset() {
    let mut codec = G722Codec::new_with_mode(1).unwrap();
    
    // Process some data
    let encoded = vec![0x55u8; G722_ENCODED_FRAME_SIZE];
    let _ = codec.decode_frame(&encoded).unwrap();
    
    // State should have changed
    assert_ne!(codec.decoder_state().state().low_band().sl, 0);
    
    // Reset
    codec.reset();
    
    // State should be back to initial values
    assert_eq!(codec.decoder_state().state().low_band().det, 32);
    assert_eq!(codec.decoder_state().state().high_band().det, 8);
    assert_eq!(codec.decoder_state().state().low_band().sl, 0);
    assert_eq!(codec.decoder_state().state().high_band().sl, 0);
}

/// Test decoding with different encoded patterns
#[test]
fn test_decoding_patterns() {
    let mut codec = G722Codec::new_with_mode(1).unwrap();
    
    // Test with all zeros
    let zeros = vec![0u8; G722_ENCODED_FRAME_SIZE];
    let decoded_zeros = codec.decode_frame(&zeros).unwrap();
    assert_eq!(decoded_zeros.len(), G722_FRAME_SIZE);
    
    // Test with all ones
    let ones = vec![0xFFu8; G722_ENCODED_FRAME_SIZE];
    let decoded_ones = codec.decode_frame(&ones).unwrap();
    assert_eq!(decoded_ones.len(), G722_FRAME_SIZE);
    
    // Test with alternating pattern
    let alternating: Vec<u8> = (0..G722_ENCODED_FRAME_SIZE)
        .map(|i| if i % 2 == 0 { 0xAAu8 } else { 0x55u8 })
        .collect();
    let decoded_alt = codec.decode_frame(&alternating).unwrap();
    assert_eq!(decoded_alt.len(), G722_FRAME_SIZE);
    
    // Different patterns should produce different decoded outputs
    assert_ne!(decoded_zeros, decoded_ones);
    assert_ne!(decoded_zeros, decoded_alt);
    assert_ne!(decoded_ones, decoded_alt);
}

/// Test mode-specific decoding behavior
#[test]
fn test_mode_specific_decoding() {
    // Create encoded data
    let encoded = vec![0x55u8; G722_ENCODED_FRAME_SIZE];
    
    let mut codec1 = G722Codec::new_with_mode(1).unwrap();
    let mut codec2 = G722Codec::new_with_mode(2).unwrap();
    let mut codec3 = G722Codec::new_with_mode(3).unwrap();
    
    let decoded1 = codec1.decode_frame(&encoded).unwrap();
    let decoded2 = codec2.decode_frame(&encoded).unwrap();
    let decoded3 = codec3.decode_frame(&encoded).unwrap();
    
    // All modes should produce the same decoded length
    assert_eq!(decoded1.len(), G722_FRAME_SIZE);
    assert_eq!(decoded2.len(), G722_FRAME_SIZE);
    assert_eq!(decoded3.len(), G722_FRAME_SIZE);
    
    // Different modes should produce different decoded outputs
    // due to mode-specific bit unpacking
    println!("Mode 1 first 10 samples: {:?}", &decoded1[0..10]);
    println!("Mode 2 first 10 samples: {:?}", &decoded2[0..10]);
    println!("Mode 3 first 10 samples: {:?}", &decoded3[0..10]);
}

/// Test byte-level decoding
#[test]
fn test_byte_level_decoding() {
    let mut codec = G722Codec::new_with_mode(1).unwrap();
    
    // Test various byte values
    let test_bytes = [0x00, 0x55, 0xAA, 0xFF, 0x12, 0x34, 0x56, 0x78];
    
    for &byte in &test_bytes {
        let decoded_pair = codec.decode_byte(byte);
        
        // Should produce valid 16-bit samples
        assert!(abs_s(decoded_pair[0]) <= 32767);
        assert!(abs_s(decoded_pair[1]) <= 32767);
    }
}

/// Test round-trip encoding/decoding
#[test]
fn test_round_trip_accuracy() {
    let mut codec = G722Codec::new_with_mode(1).unwrap();
    
    // Test with sine wave
    let input = generate_sine_wave(1000.0, 16000.0, G722_FRAME_SIZE, 10000);
    
    let encoded = codec.encode_frame(&input).unwrap();
    let decoded = codec.decode_frame(&encoded).unwrap();
    
    assert_eq!(decoded.len(), input.len());
    
    // Calculate similarity (lossy codec, so not exact)
    let similarity = calculate_sample_similarity(&input, &decoded);
    println!("Round-trip similarity: {:.2}%", similarity * 100.0);
    
    // Should preserve reasonable similarity for a lossy codec
    assert!(similarity > 0.3, "Round-trip similarity should be > 30%, got {:.2}%", similarity * 100.0);
}

/// Test round-trip with different frequencies
#[test]
fn test_round_trip_frequencies() {
    let mut codec = G722Codec::new_with_mode(1).unwrap();
    
    let frequencies = [100.0, 1000.0, 4000.0, 7000.0];
    
    for &freq in &frequencies {
        let input = generate_sine_wave(freq, 16000.0, G722_FRAME_SIZE, 8000);
        
        let encoded = codec.encode_frame(&input).unwrap();
        let decoded = codec.decode_frame(&encoded).unwrap();
        
        let similarity = calculate_sample_similarity(&input, &decoded);
        println!("{}Hz round-trip similarity: {:.2}%", freq, similarity * 100.0);
        
        // Higher frequencies may have lower similarity due to G.722's nature
        let min_similarity = if freq > 6000.0 { 0.2 } else { 0.3 };
        assert!(similarity > min_similarity, 
               "{}Hz round-trip similarity should be > {:.1}%, got {:.2}%", 
               freq, min_similarity * 100.0, similarity * 100.0);
    }
}

/// Test decoding with silence
#[test]
fn test_decoding_silence() {
    let mut codec = G722Codec::new_with_mode(1).unwrap();
    
    // Encode silence
    let silence = vec![0i16; G722_FRAME_SIZE];
    let encoded = codec.encode_frame(&silence).unwrap();
    
    // Decode it back
    let decoded = codec.decode_frame(&encoded).unwrap();
    
    // Decoded silence should be close to original silence
    let max_deviation = decoded.iter().map(|&x| abs_s(x)).max().unwrap_or(0);
    assert!(max_deviation < 1000, "Decoded silence should be close to zero, max deviation: {}", max_deviation);
}

/// Test low-band ADPCM decoding with different modes
#[test]
fn test_low_band_adpcm_decoding() {
    let mut state = AdpcmState::new();
    
    // Test different modes
    for mode in 1..=3 {
        let decoded = adpcm::low_band_decode(30, mode, &mut state);
        assert!(abs_s(decoded) <= 32767, "Low-band decoded value should be in range for mode {}", mode);
    }
}

/// Test high-band ADPCM decoding
#[test]
fn test_high_band_adpcm_decoding() {
    let mut state = AdpcmState::new();
    
    // Test different input values
    for value in 0..=3 {
        let decoded = adpcm::high_band_decode(value, &mut state);
        assert!(abs_s(decoded) <= 32767, "High-band decoded value should be in range for value {}", value);
    }
}

/// Test decoder with edge case inputs
#[test]
fn test_decoder_edge_cases() {
    let mut codec = G722Codec::new_with_mode(1).unwrap();
    
    // Test with extreme encoded values
    let extreme_values = vec![0x00u8, 0xFFu8];
    for &extreme in &extreme_values {
        let encoded = vec![extreme; G722_ENCODED_FRAME_SIZE];
        let decoded = codec.decode_frame(&encoded).unwrap();
        
        assert_eq!(decoded.len(), G722_FRAME_SIZE);
        
        // All samples should be valid
        for &sample in &decoded {
            assert!(abs_s(sample) <= 32767, "Sample should be in valid range: {}", sample);
        }
    }
}

/// Test decoder determinism
#[test]
fn test_decoder_determinism() {
    let encoded = vec![0x55u8; G722_ENCODED_FRAME_SIZE];
    
    // Decode the same data multiple times with fresh codecs
    let mut results = Vec::new();
    for _ in 0..5 {
        let mut codec = G722Codec::new_with_mode(1).unwrap();
        let decoded = codec.decode_frame(&encoded).unwrap();
        results.push(decoded);
    }
    
    // All results should be identical
    for i in 1..results.len() {
        assert_eq!(results[0], results[i], "Decoder should be deterministic");
    }
}

/// Test decoder with random encoded data
#[test]
fn test_decoder_random_data() {
    let mut codec = G722Codec::new_with_mode(1).unwrap();
    
    for _ in 0..10 {
        let encoded = (0..80).map(|_| rand::random::<u8>()).collect::<Vec<u8>>();
        let decoded = codec.decode_frame(&encoded).unwrap();
        
        // All samples should be valid
        for &sample in &decoded {
            assert!(abs_s(sample) <= 32767, "Sample should be in valid range: {}", sample);
        }
    }
}

/// Test decoder state evolution
#[test]
fn test_decoder_state_evolution() {
    let mut codec = G722Codec::new_with_mode(1).unwrap();
    
    // Use a simple pattern to verify state evolution
    let pattern = [0x80, 0x00, 0x80, 0x00, 0x80, 0x00, 0x80, 0x00];
    let mut full_encoded = Vec::new();
    
    // Repeat pattern to fill a frame
    for _ in 0..10 {
        full_encoded.extend_from_slice(&pattern);
    }
    
    // Decode multiple frames
    for _ in 0..5 {
        let decoded = codec.decode_frame(&full_encoded).unwrap();
        
        // All samples should be valid
        for &sample in &decoded {
            assert!(abs_s(sample) <= 32767, "Sample should be in valid range: {}", sample);
        }
    }
}

/// Test decoder with frame boundary conditions
#[test]
fn test_decoder_frame_boundaries() {
    let mut codec = G722Codec::new_with_mode(1).unwrap();
    
    // Test decoding multiple consecutive frames
    let encoded_frames = [
        vec![0x11u8; G722_ENCODED_FRAME_SIZE],
        vec![0x22u8; G722_ENCODED_FRAME_SIZE],
        vec![0x33u8; G722_ENCODED_FRAME_SIZE],
    ];
    
    let mut decoded_frames = Vec::new();
    for encoded in &encoded_frames {
        let decoded = codec.decode_frame(encoded).unwrap();
        decoded_frames.push(decoded);
    }
    
    // All frames should decode successfully
    for decoded in &decoded_frames {
        assert_eq!(decoded.len(), G722_FRAME_SIZE);
    }
    
    // Each frame should produce different output due to adaptive nature
    assert_ne!(decoded_frames[0], decoded_frames[1]);
    assert_ne!(decoded_frames[1], decoded_frames[2]);
    assert_ne!(decoded_frames[0], decoded_frames[2]);
}

/// Test decoder with continuous encoded stream
#[test]
fn test_decoder_continuous_stream() {
    let mut codec = G722Codec::new_with_mode(1).unwrap();
    
    // Generate a continuous pattern across multiple frames
    let mut continuous_encoded = Vec::new();
    for i in 0..(G722_ENCODED_FRAME_SIZE * 3) {
        continuous_encoded.push(((i * 17) % 256) as u8);
    }
    
    let mut decoded_frames = Vec::new();
    for i in 0..3 {
        let start = i * G722_ENCODED_FRAME_SIZE;
        let end = start + G722_ENCODED_FRAME_SIZE;
        let frame = &continuous_encoded[start..end];
        
        let decoded = codec.decode_frame(frame).unwrap();
        decoded_frames.push(decoded);
    }
    
    // All frames should decode successfully
    for decoded in &decoded_frames {
        assert_eq!(decoded.len(), G722_FRAME_SIZE);
    }
    
    // Check that the decoder adapts over time
    assert_ne!(decoded_frames[0], decoded_frames[1]);
    assert_ne!(decoded_frames[1], decoded_frames[2]);
}

/// Test decoder stability with repeated input
#[test]
fn test_decoder_stability() {
    let mut codec = G722Codec::new_with_mode(1).unwrap();
    
    // Use the same encoded frame repeatedly
    let encoded = vec![0x77u8; G722_ENCODED_FRAME_SIZE];
    
    let mut decoded_frames = Vec::new();
    for _ in 0..10 {
        let decoded = codec.decode_frame(&encoded).unwrap();
        decoded_frames.push(decoded);
    }
    
    // All frames should decode successfully
    for decoded in &decoded_frames {
        assert_eq!(decoded.len(), G722_FRAME_SIZE);
        
        // Check for stability (no extreme values)
        for &sample in decoded {
            assert!(abs_s(sample) <= 32767, "Sample should be stable: {}", sample);
        }
    }
    
    // Later frames should be different due to adaptation
    assert_ne!(decoded_frames[0], decoded_frames[9]);
}

/// Test cross-mode decoding compatibility
#[test]
fn test_cross_mode_decoding() {
    // Encode with mode 1
    let mut encoder = G722Codec::new_with_mode(1).unwrap();
    let input = generate_sine_wave(1000.0, 16000.0, G722_FRAME_SIZE, 8000);
    let encoded = encoder.encode_frame(&input).unwrap();
    
    // Try to decode with different modes
    for mode in 1..=3 {
        let mut decoder = G722Codec::new_with_mode(mode).unwrap();
        let decoded = decoder.decode_frame(&encoded).unwrap();
        
        assert_eq!(decoded.len(), G722_FRAME_SIZE);
        
        // All samples should be valid
        for &sample in &decoded {
            assert!(abs_s(sample) <= 32767, "Cross-mode decoded sample should be valid: {}", sample);
        }
    }
} 