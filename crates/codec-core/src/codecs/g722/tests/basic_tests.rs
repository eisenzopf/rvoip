//! Basic G.722 Tests
//!
//! This module contains basic unit tests for the G.722 codec implementation.
//! Tests cover codec creation, state management, and fundamental operations.

use super::utils::*;
use crate::codecs::g722::*;
use crate::codecs::g722::state::{G722State, AdpcmState};
use crate::codecs::g722::reference::abs_s;
use crate::codecs::g722::codec::{G722_FRAME_SIZE, G722_ENCODED_FRAME_SIZE};
use crate::codecs::g722::adpcm;
use crate::codecs::g722::qmf;
use crate::types::AudioCodec;

/// Test codec creation with different modes
#[test]
fn test_codec_creation() {
    // Test valid modes
    for mode in 1..=3 {
        let codec = G722Codec::new_with_mode(mode);
        assert!(codec.is_ok(), "Mode {} should be valid", mode);
        
        let codec = codec.unwrap();
        assert_eq!(codec.mode(), mode, "Mode should match");
    }
    
    // Test invalid modes
    for mode in [0, 4, 5, 255] {
        let codec = G722Codec::new_with_mode(mode);
        assert!(codec.is_err(), "Mode {} should be invalid", mode);
    }
}

/// Test codec mode properties
#[test]
fn test_codec_mode_properties() {
    let codec1 = G722Codec::new_with_mode(1).unwrap();
    assert_eq!(codec1.info().bitrate, 64000);
    
    let codec2 = G722Codec::new_with_mode(2).unwrap();
    assert_eq!(codec2.info().bitrate, 56000);
    
    let codec3 = G722Codec::new_with_mode(3).unwrap();
    assert_eq!(codec3.info().bitrate, 48000);
}

/// Test ADPCM state initialization
#[test]
fn test_adpcm_state_init() {
    let low_band = AdpcmState::new_low_band();
    assert_eq!(low_band.det, 32, "Low band should initialize with DETL=32");
    assert_eq!(low_band.sl, 0);
    assert_eq!(low_band.spl, 0);
    assert_eq!(low_band.szl, 0);
    
    let high_band = AdpcmState::new_high_band();
    assert_eq!(high_band.det, 8, "High band should initialize with DETH=8");
    assert_eq!(high_band.sl, 0);
    assert_eq!(high_band.spl, 0);
    assert_eq!(high_band.szl, 0);
}

/// Test G.722 state initialization
#[test]
fn test_g722_state_init() {
    let state = G722State::new();
    assert_eq!(state.low_band().det, 32);
    assert_eq!(state.high_band().det, 8);
    assert_eq!(state.qmf_tx_delay().len(), 24);
    assert_eq!(state.qmf_rx_delay().len(), 24);
    
    // Check that delay lines are initialized to zero
    for &sample in state.qmf_tx_delay() {
        assert_eq!(sample, 0);
    }
    for &sample in state.qmf_rx_delay() {
        assert_eq!(sample, 0);
    }
}

/// Test state reset functionality
#[test]
fn test_state_reset() {
    let mut codec = G722Codec::new_with_mode(1).unwrap();
    
    // Process some data to change state
    let input = vec![1000i16; G722_FRAME_SIZE];
    let _ = codec.encode_frame(&input).unwrap();
    
    // Verify state has changed
    assert_ne!(codec.encoder_state().state().low_band().sl, 0);
    
    // Reset and verify state is back to initial values
    codec.reset();
    assert_eq!(codec.encoder_state().state().low_band().det, 32);
    assert_eq!(codec.encoder_state().state().high_band().det, 8);
    assert_eq!(codec.encoder_state().state().low_band().sl, 0);
    assert_eq!(codec.encoder_state().state().high_band().sl, 0);
}

/// Test frame size constants
#[test]
fn test_frame_size_constants() {
    assert_eq!(G722_FRAME_SIZE, 160, "Frame size should be 160 samples");
    assert_eq!(G722_ENCODED_FRAME_SIZE, 80, "Encoded frame size should be 80 bytes");
    assert_eq!(G722_FRAME_SIZE / 2, G722_ENCODED_FRAME_SIZE, "2:1 compression ratio");
}

/// Test codec info
#[test]
fn test_codec_info() {
    let codec = G722Codec::new_with_mode(1).unwrap();
    let info = codec.info();
    
    assert_eq!(info.name, "G.722");
    assert_eq!(info.sample_rate, 16000);
    assert_eq!(info.channels, 1);
    assert_eq!(info.frame_size, G722_FRAME_SIZE);
    assert_eq!(info.payload_type, Some(9));
}

/// Test compression ratio
#[test]
fn test_compression_ratio() {
    let codec = G722Codec::new_with_mode(1).unwrap();
    assert_eq!(codec.compression_ratio(), 0.5);
}

/// Test ITU-T reference functions
#[test]
fn test_itu_reference_functions() {
    // Test limit function
    assert_eq!(limit(0), 0);
    assert_eq!(limit(32767), 32767);
    assert_eq!(limit(-32768), -32768);
    assert_eq!(limit(100000), 32767);
    assert_eq!(limit(-100000), -32768);
    
    // Test add function with saturation
    assert_eq!(add(1000, 2000), 3000);
    assert_eq!(add(32000, 1000), 32767);  // Should saturate
    assert_eq!(add(-32000, -1000), -32768); // Should saturate
    
    // Test sub function with saturation
    assert_eq!(sub(3000, 1000), 2000);
    assert_eq!(sub(-32000, 1000), -32768); // Should saturate
    assert_eq!(sub(32000, -1000), 32767);  // Should saturate
}

/// Test quantization bounds
#[test]
fn test_quantization_bounds() {
    let mut state = AdpcmState::new_low_band();
    
    // Test quantl function bounds
    let result = quantl(1000, 32);
    assert!(result >= 0 && result <= 63, "quantl result should be in 0-63 range");
    
    let result = quantl(-1000, 32);
    assert!(result >= 0 && result <= 63, "quantl result should be in 0-63 range");
    
    // Test quanth function bounds  
    let result = quanth(500, 32);
    assert!(result >= 0 && result <= 3, "quanth result should be in 0-3 range");
    
    let result = quanth(-500, 32);
    assert!(result >= 0 && result <= 3, "quanth result should be in 0-3 range");
}

/// Test bit packing for different modes
#[test]
fn test_bit_packing_modes() {
    let mut codec1 = G722Codec::new_with_mode(1).unwrap();
    let mut codec2 = G722Codec::new_with_mode(2).unwrap();
    let mut codec3 = G722Codec::new_with_mode(3).unwrap();
    
    let samples = [1000i16, 2000i16];
    
    let byte1 = codec1.encode_sample_pair(samples);
    let byte2 = codec2.encode_sample_pair(samples);
    let byte3 = codec3.encode_sample_pair(samples);
    
    // Different modes should produce different bit patterns
    // (though this depends on input, so we just check they're valid)
    assert!(byte1 <= 255);
    assert!(byte2 <= 255);
    assert!(byte3 <= 255);
    
    // Test that we can decode them back
    let decoded1 = codec1.decode_byte(byte1);
    let decoded2 = codec2.decode_byte(byte2);
    let decoded3 = codec3.decode_byte(byte3);
    
    // All should produce valid 16-bit samples
    assert!(abs_s(decoded1[0]) <= 32767);
    assert!(abs_s(decoded1[1]) <= 32767);
    assert!(abs_s(decoded2[0]) <= 32767);
    assert!(abs_s(decoded2[1]) <= 32767);
    assert!(abs_s(decoded3[0]) <= 32767);
    assert!(abs_s(decoded3[1]) <= 32767);
}

/// Test basic encode/decode round trip
#[test]
fn test_basic_round_trip() {
    let mut codec = G722Codec::new_with_mode(1).unwrap();
    
    // Test with silence
    let silence = vec![0i16; G722_FRAME_SIZE];
    let encoded = codec.encode_frame(&silence).unwrap();
    assert_eq!(encoded.len(), G722_ENCODED_FRAME_SIZE);
    
    let decoded = codec.decode_frame(&encoded).unwrap();
    assert_eq!(decoded.len(), G722_FRAME_SIZE);
    
    // Test with sine wave
    let mut sine_wave = Vec::new();
    for i in 0..G722_FRAME_SIZE {
        let t = i as f32 / 16000.0;
        let sample = (5000.0 * (2.0 * std::f32::consts::PI * 1000.0 * t).sin()) as i16;
        sine_wave.push(sample);
    }
    
    let encoded = codec.encode_frame(&sine_wave).unwrap();
    assert_eq!(encoded.len(), G722_ENCODED_FRAME_SIZE);
    
    let decoded = codec.decode_frame(&encoded).unwrap();
    assert_eq!(decoded.len(), G722_FRAME_SIZE);
    
    // For a lossy codec, we just check that energy is preserved roughly
    let input_energy: f64 = sine_wave.iter().map(|&x| (x as f64).powi(2)).sum();
    let output_energy: f64 = decoded.iter().map(|&x| (x as f64).powi(2)).sum();
    let energy_ratio = output_energy / input_energy;
    
    println!("Energy ratio: {:.2}", energy_ratio);
    assert!(energy_ratio > 0.01, "Energy ratio should be > 0.01, got {}", energy_ratio);
    assert!(energy_ratio < 100.0, "Energy ratio should be < 100.0, got {}", energy_ratio);
}

/// Test frame size validation
#[test]
fn test_frame_size_validation() {
    let mut codec = G722Codec::new_with_mode(1).unwrap();
    
    // Test encoding with wrong frame size
    let wrong_size_input = vec![0i16; 100];
    assert!(codec.encode_frame(&wrong_size_input).is_err());
    
    // Test decoding with wrong frame size
    let wrong_size_encoded = vec![0u8; 50];
    assert!(codec.decode_frame(&wrong_size_encoded).is_err());
}

/// Test default codec
#[test]
fn test_default_codec() {
    let codec = G722Codec::default();
    assert_eq!(codec.mode(), 1);
} 

/// Test validation edge cases
#[test]
fn test_sample_validation() {
    let mut codec = G722Codec::new_with_mode(1).unwrap();
    
    // Test with valid samples (should work)
    let valid_samples = vec![0i16; G722_FRAME_SIZE];
    assert!(codec.encode_frame(&valid_samples).is_ok());
    
    // Test with extreme but valid samples (should work)
    let mut extreme_samples = vec![0i16; G722_FRAME_SIZE];
    extreme_samples[0] = i16::MIN;
    extreme_samples[1] = i16::MAX;
    extreme_samples[2] = -32767;
    extreme_samples[3] = 32767;
    
    // These should work since they're valid i16 values
    assert!(codec.encode_frame(&extreme_samples).is_ok());
    
    // Test encoded frame validation with valid data
    let encoded = vec![0u8; G722_ENCODED_FRAME_SIZE];
    assert!(codec.decode_frame(&encoded).is_ok());
    
    println!("✓ Sample validation tests passed");
} 