//! Test to verify AudioFrame is accessible from external crates
//!
//! This test ensures that the AudioFrame type can be used by external libraries
//! like session-core and client-core.

use rvoip_media_core::AudioFrame;
use std::time::Duration;

#[test]
fn test_audio_frame_creation() {
    // Test basic AudioFrame creation
    let samples = vec![100, 200, 300, 400, 500, 600];
    let sample_rate = 8000;
    let channels = 2;
    let timestamp = 1234;
    
    let audio_frame = AudioFrame::new(samples.clone(), sample_rate, channels, timestamp);
    
    // Verify all fields are accessible
    assert_eq!(audio_frame.samples, samples);
    assert_eq!(audio_frame.sample_rate, sample_rate);
    assert_eq!(audio_frame.channels, channels);
    assert_eq!(audio_frame.timestamp, timestamp);
    
    // Verify duration is calculated correctly
    let expected_duration = Duration::from_secs_f64(3.0 / 8000.0); // 3 samples per channel at 8kHz
    assert_eq!(audio_frame.duration, expected_duration);
    
    println!("✅ AudioFrame creation works correctly");
}

#[test]
fn test_audio_frame_public_fields() {
    // Test that all public fields are accessible and mutable
    let mut audio_frame = AudioFrame::new(vec![1, 2, 3, 4], 16000, 1, 5678);
    
    // Test field access
    assert_eq!(audio_frame.samples.len(), 4);
    assert_eq!(audio_frame.sample_rate, 16000);
    assert_eq!(audio_frame.channels, 1);
    assert_eq!(audio_frame.timestamp, 5678);
    
    // Test field modification
    audio_frame.samples.push(5);
    audio_frame.timestamp = 9999;
    
    assert_eq!(audio_frame.samples.len(), 5);
    assert_eq!(audio_frame.timestamp, 9999);
    
    println!("✅ AudioFrame public fields are accessible and mutable");
}

#[test]
fn test_audio_frame_methods() {
    // Test mono frame
    let mono_frame = AudioFrame::new(vec![1, 2, 3, 4], 8000, 1, 0);
    assert_eq!(mono_frame.samples_per_channel(), 4);
    assert!(mono_frame.is_mono());
    assert!(!mono_frame.is_stereo());
    
    // Test stereo frame
    let stereo_frame = AudioFrame::new(vec![1, 2, 3, 4, 5, 6], 16000, 2, 0);
    assert_eq!(stereo_frame.samples_per_channel(), 3);
    assert!(!stereo_frame.is_mono());
    assert!(stereo_frame.is_stereo());
    
    println!("✅ AudioFrame methods work correctly");
}

#[test]
fn test_audio_frame_clone() {
    // Test that AudioFrame can be cloned
    let original = AudioFrame::new(vec![10, 20, 30], 48000, 1, 12345);
    let cloned = original.clone();
    
    assert_eq!(original.samples, cloned.samples);
    assert_eq!(original.sample_rate, cloned.sample_rate);
    assert_eq!(original.channels, cloned.channels);
    assert_eq!(original.timestamp, cloned.timestamp);
    assert_eq!(original.duration, cloned.duration);
    
    println!("✅ AudioFrame cloning works correctly");
}

#[test]
fn test_audio_frame_debug() {
    // Test that AudioFrame implements Debug
    let frame = AudioFrame::new(vec![1, 2], 8000, 1, 100);
    let debug_str = format!("{:?}", frame);
    assert!(debug_str.contains("AudioFrame"));
    assert!(debug_str.contains("samples"));
    assert!(debug_str.contains("sample_rate"));
    
    println!("✅ AudioFrame Debug implementation works correctly");
}

#[test]
fn test_audio_frame_from_prelude() {
    // Test importing from prelude
    use rvoip_media_core::prelude::AudioFrame;
    
    let frame = AudioFrame::new(vec![1, 2, 3], 8000, 1, 0);
    assert_eq!(frame.samples.len(), 3);
    
    println!("✅ AudioFrame accessible from prelude");
}

#[test]
fn test_audio_frame_realistic_scenario() {
    // Test a realistic scenario that session-core might use
    let sample_rate = 8000;
    let channels = 1;
    let frame_duration_ms = 20;
    let samples_per_frame = (sample_rate * frame_duration_ms / 1000) as usize;
    
    // Create a 20ms frame of audio samples
    let mut samples = Vec::with_capacity(samples_per_frame);
    for i in 0..samples_per_frame {
        samples.push((i as i16) * 10); // Simple test pattern
    }
    
    let timestamp = 160; // RTP timestamp for 20ms at 8kHz
    let audio_frame = AudioFrame::new(samples, sample_rate, channels, timestamp);
    
    // Verify properties
    assert_eq!(audio_frame.samples.len(), 160); // 20ms at 8kHz
    assert_eq!(audio_frame.sample_rate, 8000);
    assert_eq!(audio_frame.channels, 1);
    assert_eq!(audio_frame.timestamp, 160);
    assert_eq!(audio_frame.samples_per_channel(), 160);
    assert!(audio_frame.is_mono());
    
    // Verify duration is approximately 20ms
    let expected_duration = Duration::from_millis(20);
    let actual_duration = audio_frame.duration;
    let diff = if actual_duration > expected_duration {
        actual_duration - expected_duration
    } else {
        expected_duration - actual_duration
    };
    assert!(diff < Duration::from_micros(100), "Duration should be ~20ms");
    
    println!("✅ AudioFrame realistic scenario works correctly");
} 