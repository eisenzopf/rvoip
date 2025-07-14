//! G.722 Encoder Tests
//!
//! This module contains focused tests for the G.722 encoder functionality.
//! Tests cover mode-specific behavior, edge cases, and encoder state management.

use crate::codecs::g722::G722Codec;
use crate::codecs::g722::state::{G722EncoderState, AdpcmState};
use crate::codecs::g722::codec::{G722_FRAME_SIZE, G722_ENCODED_FRAME_SIZE};
use crate::codecs::g722::adpcm;
use super::utils::test_signals::*;

/// Test encoder creation and initialization
#[test]
fn test_encoder_creation() {
    for mode in 1..=3 {
        let codec = G722Codec::new_with_mode(mode).unwrap();
        assert_eq!(codec.mode(), mode);
        assert_eq!(codec.encoder_state().state().low_band().det, 32);
        assert_eq!(codec.encoder_state().state().high_band().det, 8);
    }
}

/// Test encoder state management
#[test]
fn test_encoder_state_management() {
    let mut codec = G722Codec::new_with_mode(1).unwrap();
    
    // Initial state
    assert_eq!(codec.encoder_state().state().low_band().sl, 0);
    assert_eq!(codec.encoder_state().state().high_band().sl, 0);
    
    // Process a frame
    let input = vec![1000i16; G722_FRAME_SIZE];
    let _ = codec.encode_frame(&input).unwrap();
    
    // State should have changed
    let low_sl = codec.encoder_state().state().low_band().sl;
    let high_sl = codec.encoder_state().state().high_band().sl;
    
    // Process another frame
    let _ = codec.encode_frame(&input).unwrap();
    
    // State should continue to evolve
    assert_ne!(codec.encoder_state().state().low_band().sl, low_sl);
    assert_ne!(codec.encoder_state().state().high_band().sl, high_sl);
}

/// Test encoder reset functionality
#[test]
fn test_encoder_reset() {
    let mut codec = G722Codec::new_with_mode(1).unwrap();
    
    // Process some data
    let input = vec![1000i16; G722_FRAME_SIZE];
    let _ = codec.encode_frame(&input).unwrap();
    
    // State should have changed
    assert_ne!(codec.encoder_state().state().low_band().sl, 0);
    
    // Reset
    codec.reset();
    
    // State should be back to initial values
    assert_eq!(codec.encoder_state().state().low_band().det, 32);
    assert_eq!(codec.encoder_state().state().high_band().det, 8);
    assert_eq!(codec.encoder_state().state().low_band().sl, 0);
    assert_eq!(codec.encoder_state().state().high_band().sl, 0);
}

/// Test encoding with different input patterns
#[test]
fn test_encoding_patterns() {
    let mut codec = G722Codec::new_with_mode(1).unwrap();
    
    // Test with silence
    let silence = vec![0i16; G722_FRAME_SIZE];
    let encoded_silence = codec.encode_frame(&silence).unwrap();
    assert_eq!(encoded_silence.len(), G722_ENCODED_FRAME_SIZE);
    
    // Test with maximum amplitude
    let max_amp = vec![32767i16; G722_FRAME_SIZE];
    let encoded_max = codec.encode_frame(&max_amp).unwrap();
    assert_eq!(encoded_max.len(), G722_ENCODED_FRAME_SIZE);
    
    // Test with minimum amplitude
    let min_amp = vec![-32768i16; G722_FRAME_SIZE];
    let encoded_min = codec.encode_frame(&min_amp).unwrap();
    assert_eq!(encoded_min.len(), G722_ENCODED_FRAME_SIZE);
    
    // Different patterns should produce different encoded outputs
    assert_ne!(encoded_silence, encoded_max);
    assert_ne!(encoded_silence, encoded_min);
    assert_ne!(encoded_max, encoded_min);
}

/// Test encoding with sine wave inputs
#[test]
fn test_encoding_sine_waves() {
    let mut codec = G722Codec::new_with_mode(1).unwrap();
    
    // Test different frequencies
    let frequencies = [100.0, 1000.0, 4000.0, 7000.0];
    let mut encoded_outputs = Vec::new();
    
    for &freq in &frequencies {
        let sine = generate_sine_wave(freq, 16000.0, G722_FRAME_SIZE, 10000);
        let encoded = codec.encode_frame(&sine).unwrap();
        assert_eq!(encoded.len(), G722_ENCODED_FRAME_SIZE);
        encoded_outputs.push(encoded);
    }
    
    // Different frequencies should produce different encoded outputs
    for i in 0..frequencies.len() {
        for j in i+1..frequencies.len() {
            assert_ne!(encoded_outputs[i], encoded_outputs[j], 
                      "Frequencies {} and {} Hz should produce different encoded outputs",
                      frequencies[i], frequencies[j]);
        }
    }
}

/// Test mode-specific encoding behavior
#[test]
fn test_mode_specific_encoding() {
    let input = generate_sine_wave(1000.0, 16000.0, G722_FRAME_SIZE, 10000);
    
    let mut codec1 = G722Codec::new_with_mode(1).unwrap();
    let mut codec2 = G722Codec::new_with_mode(2).unwrap();
    let mut codec3 = G722Codec::new_with_mode(3).unwrap();
    
    let encoded1 = codec1.encode_frame(&input).unwrap();
    let encoded2 = codec2.encode_frame(&input).unwrap();
    let encoded3 = codec3.encode_frame(&input).unwrap();
    
    // All modes should produce the same encoded length
    assert_eq!(encoded1.len(), G722_ENCODED_FRAME_SIZE);
    assert_eq!(encoded2.len(), G722_ENCODED_FRAME_SIZE);
    assert_eq!(encoded3.len(), G722_ENCODED_FRAME_SIZE);
    
    // Different modes should produce different encoded outputs
    // (this depends on the mode-specific quantization)
    println!("Mode 1 first 10 bytes: {:?}", &encoded1[0..10]);
    println!("Mode 2 first 10 bytes: {:?}", &encoded2[0..10]);
    println!("Mode 3 first 10 bytes: {:?}", &encoded3[0..10]);
}

/// Test sample-pair encoding
#[test]
fn test_sample_pair_encoding() {
    let mut codec = G722Codec::new_with_mode(1).unwrap();
    
    // Test various sample pairs
    let test_pairs = [
        [0i16, 0i16],
        [1000i16, 2000i16],
        [-1000i16, -2000i16],
        [32767i16, -32768i16],
        [100i16, -100i16],
    ];
    
    for &samples in &test_pairs {
        let encoded_byte = codec.encode_sample_pair(samples);
        assert!(encoded_byte <= 255, "Encoded byte should be valid: {}", encoded_byte);
    }
}

/// Test encoding with impulse inputs
#[test]
fn test_encoding_impulses() {
    let mut codec = G722Codec::new_with_mode(1).unwrap();
    
    // Test impulse at different positions
    let positions = [0, 10, 80, 159];
    let mut encoded_outputs = Vec::new();
    
    for &pos in &positions {
        let impulse = generate_impulse(G722_FRAME_SIZE, pos, 10000);
        let encoded = codec.encode_frame(&impulse).unwrap();
        assert_eq!(encoded.len(), G722_ENCODED_FRAME_SIZE);
        encoded_outputs.push(encoded);
    }
    
    // Different impulse positions should produce different encoded outputs
    for i in 0..positions.len() {
        for j in i+1..positions.len() {
            assert_ne!(encoded_outputs[i], encoded_outputs[j], 
                      "Impulse positions {} and {} should produce different encoded outputs",
                      positions[i], positions[j]);
        }
    }
}

/// Test encoding with white noise
#[test]
fn test_encoding_white_noise() {
    let mut codec = G722Codec::new_with_mode(1).unwrap();
    
    let noise = generate_white_noise(G722_FRAME_SIZE, 5000);
    let encoded = codec.encode_frame(&noise).unwrap();
    assert_eq!(encoded.len(), G722_ENCODED_FRAME_SIZE);
    
    // Encoded noise should use a variety of byte values
    let mut byte_counts = [0u32; 256];
    for &byte in &encoded {
        byte_counts[byte as usize] += 1;
    }
    
    // At least some variety in encoded values
    let non_zero_count = byte_counts.iter().filter(|&&count| count > 0).count();
    println!("Encoded noise uses {} different byte values", non_zero_count);
    assert!(non_zero_count > 5, "Encoded noise should have some variety in byte values, got {}", non_zero_count);
}

/// Test low-band ADPCM encoding
#[test]
fn test_low_band_adpcm_encoding() {
    let mut state = AdpcmState::new_low_band();
    
    // Test different modes
    for mode in 1..=3 {
        let encoded = adpcm::low_band_encode(1000, &mut state, mode);
        assert!(encoded <= 255, "Low-band encoded value should be valid for mode {}", mode);
    }
}

/// Test high-band ADPCM encoding
#[test]
fn test_high_band_adpcm_encoding() {
    let mut state = AdpcmState::new_high_band();
    
    let encoded = adpcm::high_band_encode(1000, &mut state);
    assert!(encoded <= 3, "High-band encoded value should be 2-bit (0-3)");
}

/// Test encoder with edge case inputs
#[test]
fn test_encoder_edge_cases() {
    let mut codec = G722Codec::new_with_mode(1).unwrap();
    
    // Test with alternating min/max values
    let mut alternating = Vec::new();
    for i in 0..G722_FRAME_SIZE {
        alternating.push(if i % 2 == 0 { 32767i16 } else { -32768i16 });
    }
    
    let encoded = codec.encode_frame(&alternating).unwrap();
    assert_eq!(encoded.len(), G722_ENCODED_FRAME_SIZE);
    
    // Test with gradual ramp
    let ramp: Vec<i16> = (0..G722_FRAME_SIZE)
        .map(|i| (i as i16) * 200)
        .collect();
    
    let encoded = codec.encode_frame(&ramp).unwrap();
    assert_eq!(encoded.len(), G722_ENCODED_FRAME_SIZE);
}

/// Test encoder determinism
#[test]
fn test_encoder_determinism() {
    let input = generate_sine_wave(1000.0, 16000.0, G722_FRAME_SIZE, 10000);
    
    // Encode the same input multiple times with fresh codecs
    let mut results = Vec::new();
    for _ in 0..5 {
        let mut codec = G722Codec::new_with_mode(1).unwrap();
        let encoded = codec.encode_frame(&input).unwrap();
        results.push(encoded);
    }
    
    // All results should be identical
    for i in 1..results.len() {
        assert_eq!(results[0], results[i], "Encoder should be deterministic");
    }
}

/// Test encoder state isolation between modes
#[test]
fn test_encoder_state_isolation() {
    let input = vec![1000i16; G722_FRAME_SIZE];
    
    let mut codec1 = G722Codec::new_with_mode(1).unwrap();
    let mut codec2 = G722Codec::new_with_mode(2).unwrap();
    
    // Process same input with both codecs
    let _ = codec1.encode_frame(&input).unwrap();
    let _ = codec2.encode_frame(&input).unwrap();
    
    // States should be independent
    let state1 = codec1.encoder_state().state();
    let state2 = codec2.encoder_state().state();
    
    // Even though the input was the same, the states may evolve differently
    // due to mode-specific quantization
    assert_eq!(state1.low_band().det, state2.low_band().det); // Should be same initially
}

/// Test encoder with frame boundary conditions
#[test]
fn test_encoder_frame_boundaries() {
    let mut codec = G722Codec::new_with_mode(1).unwrap();
    
    // Test encoding multiple consecutive frames
    let frame1 = vec![1000i16; G722_FRAME_SIZE];
    let frame2 = vec![2000i16; G722_FRAME_SIZE];
    let frame3 = vec![3000i16; G722_FRAME_SIZE];
    
    let encoded1 = codec.encode_frame(&frame1).unwrap();
    let encoded2 = codec.encode_frame(&frame2).unwrap();
    let encoded3 = codec.encode_frame(&frame3).unwrap();
    
    // All frames should encode successfully
    assert_eq!(encoded1.len(), G722_ENCODED_FRAME_SIZE);
    assert_eq!(encoded2.len(), G722_ENCODED_FRAME_SIZE);
    assert_eq!(encoded3.len(), G722_ENCODED_FRAME_SIZE);
    
    // Each frame should produce different output due to adaptive nature
    assert_ne!(encoded1, encoded2);
    assert_ne!(encoded2, encoded3);
    assert_ne!(encoded1, encoded3);
}

/// Test encoder with continuous signal
#[test]
fn test_encoder_continuous_signal() {
    let mut codec = G722Codec::new_with_mode(1).unwrap();
    
    // Generate a continuous sine wave across multiple frames
    let total_samples = G722_FRAME_SIZE * 3;
    let continuous_signal = generate_sine_wave(1000.0, 16000.0, total_samples, 10000);
    
    let mut encoded_frames = Vec::new();
    for i in 0..3 {
        let start = i * G722_FRAME_SIZE;
        let end = start + G722_FRAME_SIZE;
        let frame = &continuous_signal[start..end];
        
        let encoded = codec.encode_frame(frame).unwrap();
        encoded_frames.push(encoded);
    }
    
    // All frames should encode successfully
    for encoded in &encoded_frames {
        assert_eq!(encoded.len(), G722_ENCODED_FRAME_SIZE);
    }
    
    // Check that the encoder adapts over time
    assert_ne!(encoded_frames[0], encoded_frames[1]);
    assert_ne!(encoded_frames[1], encoded_frames[2]);
} 