//! Test AudioFrame integration in session-core
//!
//! This test verifies that:
//! 1. Session-core AudioFrame can be created and used
//! 2. Conversions between media-core and session-core AudioFrame work correctly
//! 3. AudioStreamConfig works as expected
//! 4. All utility methods function properly

use rvoip_session_core::api::types::{AudioFrame, AudioStreamConfig};
use std::time::Duration;

#[test]
fn test_session_audio_frame_creation() {
    // Test basic AudioFrame creation in session-core
    let samples = vec![100, 200, 300, 400, 500, 600];
    let sample_rate = 8000;
    let channels = 2;
    let timestamp = 1234;
    
    let audio_frame = AudioFrame::new(samples.clone(), sample_rate, channels, timestamp);
    
    // Verify all fields are correctly set
    assert_eq!(audio_frame.samples, samples);
    assert_eq!(audio_frame.sample_rate, sample_rate);
    assert_eq!(audio_frame.channels, channels);
    assert_eq!(audio_frame.timestamp, timestamp);
    
    println!("✅ Session-core AudioFrame creation works correctly");
}

#[test]
fn test_audio_frame_utility_methods() {
    // Test mono frame
    let mono_frame = AudioFrame::new(vec![1, 2, 3, 4], 8000, 1, 0);
    assert_eq!(mono_frame.samples_per_channel(), 4);
    assert!(mono_frame.is_mono());
    assert!(!mono_frame.is_stereo());
    
    // Verify duration calculation
    let expected_duration_ms = 4.0 * 1000.0 / 8000.0; // 4 samples at 8kHz
    assert!((mono_frame.duration.as_secs_f64() * 1000.0 - expected_duration_ms).abs() < 0.001);
    
    // Test stereo frame
    let stereo_frame = AudioFrame::new(vec![1, 2, 3, 4, 5, 6], 16000, 2, 0);
    assert_eq!(stereo_frame.samples_per_channel(), 3);
    assert!(!stereo_frame.is_mono());
    assert!(stereo_frame.is_stereo());
    
    // Verify duration calculation for stereo
    let expected_stereo_duration_ms = 3.0 * 1000.0 / 16000.0; // 3 samples per channel at 16kHz
    assert!((stereo_frame.duration.as_secs_f64() * 1000.0 - expected_stereo_duration_ms).abs() < 0.001);
    
    // Test Duration field
    let duration = mono_frame.duration;
    let expected_duration = Duration::from_secs_f64(expected_duration_ms / 1000.0);
    assert!((duration.as_secs_f64() - expected_duration.as_secs_f64()).abs() < 0.001);
    
    println!("✅ AudioFrame utility methods work correctly");
}

#[test]
fn test_audio_frame_conversion_media_to_session() {
    // Create a media-core AudioFrame
    let samples = vec![10, 20, 30, 40];
    let sample_rate = 16000;
    let channels = 1;
    let timestamp = 5678;
    
    let media_frame = rvoip_media_core::AudioFrame::new(samples.clone(), sample_rate, channels, timestamp);
    
    // Convert to session-core AudioFrame
    let session_frame: AudioFrame = media_frame.into();
    
    // Verify conversion preserves all data
    assert_eq!(session_frame.samples, samples);
    assert_eq!(session_frame.sample_rate, sample_rate);
    assert_eq!(session_frame.channels, channels);
    assert_eq!(session_frame.timestamp, timestamp);
    
    println!("✅ Media-core to session-core AudioFrame conversion works correctly");
}

#[test]
fn test_audio_frame_conversion_session_to_media() {
    // Create a session-core AudioFrame
    let samples = vec![100, 200, 300, 400];
    let sample_rate = 48000;
    let channels = 2;
    let timestamp = 9999;
    
    let session_frame = AudioFrame::new(samples.clone(), sample_rate, channels, timestamp);
    
    // Convert to media-core AudioFrame
    let media_frame: rvoip_media_core::AudioFrame = session_frame.into();
    
    // Verify conversion preserves all data
    assert_eq!(media_frame.samples, samples);
    assert_eq!(media_frame.sample_rate, sample_rate);
    assert_eq!(media_frame.channels, channels);
    assert_eq!(media_frame.timestamp, timestamp);
    
    println!("✅ Session-core to media-core AudioFrame conversion works correctly");
}

#[test]
fn test_audio_frame_round_trip_conversion() {
    // Test that converting back and forth preserves data
    let original_samples = vec![1, 2, 3, 4, 5, 6, 7, 8];
    let original_sample_rate = 32000;
    let original_channels = 2;
    let original_timestamp = 12345;
    
    // Start with media-core frame
    let media_frame = rvoip_media_core::AudioFrame::new(
        original_samples.clone(),
        original_sample_rate,
        original_channels,
        original_timestamp,
    );
    
    // Convert to session-core and back
    let session_frame: AudioFrame = media_frame.into();
    let converted_media_frame: rvoip_media_core::AudioFrame = session_frame.into();
    
    // Verify round-trip conversion preserves all data
    assert_eq!(converted_media_frame.samples, original_samples);
    assert_eq!(converted_media_frame.sample_rate, original_sample_rate);
    assert_eq!(converted_media_frame.channels, original_channels);
    assert_eq!(converted_media_frame.timestamp, original_timestamp);
    
    println!("✅ Round-trip AudioFrame conversion works correctly");
}

#[test]
fn test_audio_stream_config_creation() {
    // Test default configuration
    let default_config = AudioStreamConfig::default();
    assert_eq!(default_config.sample_rate, 8000);
    assert_eq!(default_config.channels, 1);
    assert_eq!(default_config.codec, "PCMU");
    assert_eq!(default_config.frame_size_ms, 20);
    assert!(default_config.enable_aec);
    assert!(default_config.enable_agc);
    assert!(default_config.enable_vad);
    
    // Test custom configuration
    let custom_config = AudioStreamConfig::new(16000, 2, "Opus");
    assert_eq!(custom_config.sample_rate, 16000);
    assert_eq!(custom_config.channels, 2);
    assert_eq!(custom_config.codec, "Opus");
    assert_eq!(custom_config.frame_size_ms, 20); // Default
    
    println!("✅ AudioStreamConfig creation works correctly");
}

#[test]
fn test_audio_stream_config_presets() {
    // Test telephony preset
    let telephony = AudioStreamConfig::telephony();
    assert_eq!(telephony.sample_rate, 8000);
    assert_eq!(telephony.channels, 1);
    assert_eq!(telephony.codec, "PCMU");
    
    // Test wideband preset
    let wideband = AudioStreamConfig::wideband();
    assert_eq!(wideband.sample_rate, 16000);
    assert_eq!(wideband.channels, 1);
    assert_eq!(wideband.codec, "Opus");
    
    // Test high-quality preset
    let high_quality = AudioStreamConfig::high_quality();
    assert_eq!(high_quality.sample_rate, 48000);
    assert_eq!(high_quality.channels, 2);
    assert_eq!(high_quality.codec, "Opus");
    
    println!("✅ AudioStreamConfig presets work correctly");
}

#[test]
fn test_audio_stream_config_utility_methods() {
    // Test frame size calculations
    let config = AudioStreamConfig::new(8000, 1, "PCMU");
    
    // 20ms at 8kHz = 160 samples
    assert_eq!(config.frame_size_samples(), 160);
    // 160 samples * 1 channel * 2 bytes (16-bit) = 320 bytes
    assert_eq!(config.frame_size_bytes(), 320);
    
    // Test with different sample rate
    let config_16k = AudioStreamConfig::new(16000, 2, "Opus");
    
    // 20ms at 16kHz = 320 samples per channel
    assert_eq!(config_16k.frame_size_samples(), 320);
    // 320 samples * 2 channels * 2 bytes = 1280 bytes
    assert_eq!(config_16k.frame_size_bytes(), 1280);
    
    println!("✅ AudioStreamConfig utility methods work correctly");
}

#[test]
fn test_audio_frame_clone_and_debug() {
    // Test that AudioFrame can be cloned
    let original = AudioFrame::new(vec![1, 2, 3], 8000, 1, 100);
    let cloned = original.clone();
    
    assert_eq!(original.samples, cloned.samples);
    assert_eq!(original.sample_rate, cloned.sample_rate);
    assert_eq!(original.channels, cloned.channels);
    assert_eq!(original.timestamp, cloned.timestamp);
    
    // Test that AudioFrame implements Debug
    let debug_str = format!("{:?}", original);
    assert!(debug_str.contains("AudioFrame"));
    assert!(debug_str.contains("samples"));
    
    println!("✅ AudioFrame Clone and Debug work correctly");
}

#[test]
fn test_audio_stream_config_clone_and_debug() {
    // Test that AudioStreamConfig can be cloned
    let original = AudioStreamConfig::wideband();
    let cloned = original.clone();
    
    assert_eq!(original.sample_rate, cloned.sample_rate);
    assert_eq!(original.channels, cloned.channels);
    assert_eq!(original.codec, cloned.codec);
    
    // Test that AudioStreamConfig implements Debug
    let debug_str = format!("{:?}", original);
    assert!(debug_str.contains("AudioStreamConfig"));
    assert!(debug_str.contains("sample_rate"));
    
    println!("✅ AudioStreamConfig Clone and Debug work correctly");
}

#[test]
fn test_realistic_audio_streaming_scenario() {
    // Test a realistic scenario that client-core might use
    let config = AudioStreamConfig::telephony();
    
    // Create a 20ms frame of audio data
    let samples_per_frame = config.frame_size_samples();
    let mut samples = Vec::with_capacity(samples_per_frame);
    
    // Generate a simple sine wave pattern
    for i in 0..samples_per_frame {
        let sample = (i as f64 * 2.0 * std::f64::consts::PI * 440.0 / config.sample_rate as f64).sin();
        samples.push((sample * 1000.0) as i16);
    }
    
    // Create AudioFrame
    let session_frame = AudioFrame::new(samples, config.sample_rate, config.channels, 0);
    
    // Verify properties
    assert_eq!(session_frame.samples.len(), samples_per_frame);
    assert_eq!(session_frame.sample_rate, config.sample_rate);
    assert_eq!(session_frame.channels, config.channels);
    assert!(session_frame.is_mono());
    assert!(!session_frame.is_stereo());
    
    // Verify duration is approximately 20ms
    let expected_duration_ms = 20.0;
    assert!((session_frame.duration.as_secs_f64() * 1000.0 - expected_duration_ms).abs() < 0.1);
    
    // Test conversion to media-core frame
    let media_frame: rvoip_media_core::AudioFrame = session_frame.clone().into();
    assert_eq!(media_frame.samples.len(), samples_per_frame);
    assert_eq!(media_frame.sample_rate, config.sample_rate);
    
    println!("✅ Realistic audio streaming scenario works correctly");
} 