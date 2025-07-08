//! Audio Device Integration Tests
//!
//! This module tests the audio device abstraction layer including:
//! - AudioDeviceManager functionality
//! - Mock audio device implementations
//! - Session-core integration
//! - Audio streaming pipelines

use std::sync::Arc;
use std::time::Duration;
use tokio::time::timeout;

use rvoip_client_core::audio::{
    AudioDeviceManager, AudioDirection, AudioFormat, AudioError,
    AudioDevice, AudioDeviceInfo,
};
use rvoip_client_core::audio::device::AudioFrame;
use rvoip_client_core::call::CallId;

#[tokio::test]
async fn test_audio_device_manager_creation() {
    let manager = AudioDeviceManager::new().await;
    assert!(manager.is_ok(), "AudioDeviceManager creation should succeed");
}

#[tokio::test]
async fn test_list_mock_devices() {
    let manager = AudioDeviceManager::new().await.unwrap();
    
    // Test listing input devices
    let input_devices = manager.list_devices(AudioDirection::Input).await.unwrap();
    assert!(!input_devices.is_empty(), "Should have at least one input device");
    
    let mic = &input_devices[0];
    assert_eq!(mic.direction, AudioDirection::Input);
    assert_eq!(mic.id, "mock_microphone");
    assert_eq!(mic.name, "Mock Microphone");
    assert!(mic.is_default);
    
    // Test listing output devices
    let output_devices = manager.list_devices(AudioDirection::Output).await.unwrap();
    assert!(!output_devices.is_empty(), "Should have at least one output device");
    
    let speaker = &output_devices[0];
    assert_eq!(speaker.direction, AudioDirection::Output);
    assert_eq!(speaker.id, "mock_speaker");
    assert_eq!(speaker.name, "Mock Speaker");
    assert!(speaker.is_default);
}

#[tokio::test]
async fn test_get_default_devices() {
    let manager = AudioDeviceManager::new().await.unwrap();
    
    // Test getting default input device
    let input_device = manager.get_default_device(AudioDirection::Input).await.unwrap();
    assert_eq!(input_device.info().direction, AudioDirection::Input);
    assert_eq!(input_device.info().id, "mock_microphone");
    
    // Test getting default output device
    let output_device = manager.get_default_device(AudioDirection::Output).await.unwrap();
    assert_eq!(output_device.info().direction, AudioDirection::Output);
    assert_eq!(output_device.info().id, "mock_speaker");
}

#[tokio::test]
async fn test_create_specific_device() {
    let manager = AudioDeviceManager::new().await.unwrap();
    
    // Test creating specific devices
    let mic = manager.create_device("mock_microphone").await.unwrap();
    assert_eq!(mic.info().id, "mock_microphone");
    
    let speaker = manager.create_device("mock_speaker").await.unwrap();
    assert_eq!(speaker.info().id, "mock_speaker");
    
    // Test creating non-existent device
    let result = manager.create_device("non_existent_device").await;
    assert!(result.is_err());
    match result.unwrap_err() {
        AudioError::DeviceNotFound { device_id } => {
            assert_eq!(device_id, "non_existent_device");
        }
        other => panic!("Expected DeviceNotFound error, got: {:?}", other),
    }
}

#[tokio::test]
async fn test_audio_format_creation() {
    let format = AudioFormat::default_voip();
    assert_eq!(format.sample_rate, 8000);
    assert_eq!(format.channels, 1);
    assert_eq!(format.bits_per_sample, 16);
    assert_eq!(format.frame_size_ms, 20);
    assert_eq!(format.samples_per_frame(), 160); // 8000 * 20 / 1000
    assert_eq!(format.bytes_per_frame(), 320); // 160 * 1 * 2
    
    let wideband = AudioFormat::wideband_voip();
    assert_eq!(wideband.sample_rate, 16000);
    assert_eq!(wideband.channels, 1);
    assert_eq!(wideband.samples_per_frame(), 320); // 16000 * 20 / 1000
}

#[tokio::test]
async fn test_audio_frame_conversion() {
    let format = AudioFormat::default_voip();
    let samples = vec![100, 200, 300, 400];
    let timestamp = 1234567890;
    
    // Create client-core AudioFrame
    let client_frame = AudioFrame::new(samples.clone(), format.clone(), timestamp);
    
    // Convert to session-core AudioFrame
    let session_frame = client_frame.to_session_core();
    assert_eq!(session_frame.samples, samples);
    assert_eq!(session_frame.sample_rate, format.sample_rate);
    assert_eq!(session_frame.channels, format.channels as u8);
    assert_eq!(session_frame.timestamp, (timestamp / 1000) as u32);
    
    // Convert back to client-core AudioFrame
    let converted_frame = AudioFrame::from_session_core(&session_frame, format.frame_size_ms);
    assert_eq!(converted_frame.samples, samples);
    assert_eq!(converted_frame.format.sample_rate, format.sample_rate);
    assert_eq!(converted_frame.format.channels, format.channels);
    assert_eq!(converted_frame.timestamp_ms, (session_frame.timestamp as u64) * 1000);
}

#[tokio::test]
async fn test_mock_device_capture() {
    let manager = AudioDeviceManager::new().await.unwrap();
    let device = manager.get_default_device(AudioDirection::Input).await.unwrap();
    
    // Verify device is not active initially
    assert!(!device.is_active());
    assert!(device.current_format().is_none());
    
    // Start capture
    let format = AudioFormat::default_voip();
    let mut receiver = device.start_capture(format.clone()).await.unwrap();
    
    // Verify device is now active
    assert!(device.is_active());
    assert_eq!(device.current_format().unwrap(), format);
    
    // Receive a few frames
    let frame1 = timeout(Duration::from_millis(100), receiver.recv()).await;
    assert!(frame1.is_ok(), "Should receive first frame");
    let frame1 = frame1.unwrap().unwrap();
    
    assert_eq!(frame1.format, format);
    assert_eq!(frame1.samples.len(), format.samples_per_frame());
    
    let frame2 = timeout(Duration::from_millis(100), receiver.recv()).await;
    assert!(frame2.is_ok(), "Should receive second frame");
    let frame2 = frame2.unwrap().unwrap();
    
    // Verify timestamps are increasing
    assert!(frame2.timestamp_ms > frame1.timestamp_ms);
    
    // Stop capture
    device.stop_capture().await.unwrap();
    
    // Verify device is no longer active
    assert!(!device.is_active());
    assert!(device.current_format().is_none());
}

#[tokio::test]
async fn test_mock_device_playback() {
    let manager = AudioDeviceManager::new().await.unwrap();
    let device = manager.get_default_device(AudioDirection::Output).await.unwrap();
    
    // Verify device is not active initially
    assert!(!device.is_active());
    assert!(device.current_format().is_none());
    
    // Start playback
    let format = AudioFormat::default_voip();
    let sender = device.start_playback(format.clone()).await.unwrap();
    
    // Verify device is now active
    assert!(device.is_active());
    assert_eq!(device.current_format().unwrap(), format);
    
    // Send a few frames
    let samples = vec![100; format.samples_per_frame()];
    let frame1 = AudioFrame::new(samples.clone(), format.clone(), 0);
    let frame2 = AudioFrame::new(samples.clone(), format.clone(), 20);
    
    sender.send(frame1).await.unwrap();
    sender.send(frame2).await.unwrap();
    
    // Give time for frames to be processed
    tokio::time::sleep(Duration::from_millis(50)).await;
    
    // Stop playback
    device.stop_playback().await.unwrap();
    
    // Verify device is no longer active
    assert!(!device.is_active());
    assert!(device.current_format().is_none());
}

#[tokio::test]
async fn test_device_manager_playback_session() {
    let manager = AudioDeviceManager::new().await.unwrap();
    let call_id = CallId::new_v4();
    
    // Verify no active sessions initially
    assert!(!manager.is_playback_active(&call_id).await);
    assert_eq!(manager.get_active_playback_sessions().await.len(), 0);
    
    // Start playback
    let device = manager.get_default_device(AudioDirection::Output).await.unwrap();
    manager.start_playback(&call_id, device.clone()).await.unwrap();
    
    // Verify session is active
    assert!(manager.is_playback_active(&call_id).await);
    assert_eq!(manager.get_active_playback_sessions().await.len(), 1);
    assert_eq!(manager.get_active_playback_sessions().await[0], call_id);
    
    // Stop playback
    manager.stop_playback(&call_id).await.unwrap();
    
    // Verify session is no longer active
    assert!(!manager.is_playback_active(&call_id).await);
    assert_eq!(manager.get_active_playback_sessions().await.len(), 0);
}

#[tokio::test]
async fn test_device_manager_capture_session() {
    let manager = AudioDeviceManager::new().await.unwrap();
    let call_id = CallId::new_v4();
    
    // Verify no active sessions initially
    assert!(!manager.is_capture_active(&call_id).await);
    assert_eq!(manager.get_active_capture_sessions().await.len(), 0);
    
    // Start capture
    let device = manager.get_default_device(AudioDirection::Input).await.unwrap();
    manager.start_capture(&call_id, device.clone()).await.unwrap();
    
    // Verify session is active
    assert!(manager.is_capture_active(&call_id).await);
    assert_eq!(manager.get_active_capture_sessions().await.len(), 1);
    assert_eq!(manager.get_active_capture_sessions().await[0], call_id);
    
    // Stop capture
    manager.stop_capture(&call_id).await.unwrap();
    
    // Verify session is no longer active
    assert!(!manager.is_capture_active(&call_id).await);
    assert_eq!(manager.get_active_capture_sessions().await.len(), 0);
}

#[tokio::test]
async fn test_multiple_concurrent_sessions() {
    let manager = AudioDeviceManager::new().await.unwrap();
    let call_id1 = CallId::new_v4();
    let call_id2 = CallId::new_v4();
    
    // Start multiple sessions
    let input_device = manager.get_default_device(AudioDirection::Input).await.unwrap();
    let output_device = manager.get_default_device(AudioDirection::Output).await.unwrap();
    
    manager.start_capture(&call_id1, input_device.clone()).await.unwrap();
    manager.start_playback(&call_id1, output_device.clone()).await.unwrap();
    
    manager.start_capture(&call_id2, input_device.clone()).await.unwrap();
    manager.start_playback(&call_id2, output_device.clone()).await.unwrap();
    
    // Verify both sessions are active
    assert!(manager.is_capture_active(&call_id1).await);
    assert!(manager.is_playback_active(&call_id1).await);
    assert!(manager.is_capture_active(&call_id2).await);
    assert!(manager.is_playback_active(&call_id2).await);
    
    assert_eq!(manager.get_active_capture_sessions().await.len(), 2);
    assert_eq!(manager.get_active_playback_sessions().await.len(), 2);
    
    // Stop all sessions
    manager.stop_all_sessions().await.unwrap();
    
    // Verify all sessions are stopped
    assert!(!manager.is_capture_active(&call_id1).await);
    assert!(!manager.is_playback_active(&call_id1).await);
    assert!(!manager.is_capture_active(&call_id2).await);
    assert!(!manager.is_playback_active(&call_id2).await);
    
    assert_eq!(manager.get_active_capture_sessions().await.len(), 0);
    assert_eq!(manager.get_active_playback_sessions().await.len(), 0);
}

#[tokio::test]
async fn test_invalid_device_operations() {
    let manager = AudioDeviceManager::new().await.unwrap();
    
    // Test starting capture on output device
    let output_device = manager.get_default_device(AudioDirection::Output).await.unwrap();
    let format = AudioFormat::default_voip();
    let result = output_device.start_capture(format.clone()).await;
    assert!(result.is_err());
    
    // Test starting playback on input device
    let input_device = manager.get_default_device(AudioDirection::Input).await.unwrap();
    let result = input_device.start_playback(format.clone()).await;
    assert!(result.is_err());
}

#[tokio::test]
async fn test_device_format_support() {
    let manager = AudioDeviceManager::new().await.unwrap();
    let device = manager.get_default_device(AudioDirection::Input).await.unwrap();
    
    // Test supported formats
    let voip_format = AudioFormat::default_voip();
    assert!(device.supports_format(&voip_format));
    
    let wideband_format = AudioFormat::wideband_voip();
    assert!(device.supports_format(&wideband_format));
    
    // Test unsupported format (non-standard sample rate)
    let unsupported_format = AudioFormat::new(7000, 1, 16, 20);
    assert!(!device.supports_format(&unsupported_format));
}

#[tokio::test]
async fn test_silent_frame_generation() {
    let format = AudioFormat::default_voip();
    let timestamp = 1000;
    
    let silent_frame = AudioFrame::silent(format.clone(), timestamp);
    
    assert_eq!(silent_frame.format, format);
    assert_eq!(silent_frame.timestamp_ms, timestamp);
    assert_eq!(silent_frame.samples.len(), format.samples_per_frame());
    
    // All samples should be zero
    for sample in silent_frame.samples {
        assert_eq!(sample, 0);
    }
}

#[tokio::test]
async fn test_audio_error_display() {
    let errors = vec![
        AudioError::DeviceNotFound { device_id: "test_device".to_string() },
        AudioError::FormatNotSupported { 
            format: AudioFormat::default_voip(),
            device_id: "test_device".to_string(),
        },
        AudioError::DeviceInUse { device_id: "test_device".to_string() },
        AudioError::PlatformError { message: "Platform error".to_string() },
        AudioError::IoError { message: "IO error".to_string() },
        AudioError::ConfigurationError { message: "Config error".to_string() },
    ];
    
    for error in errors {
        let error_str = format!("{}", error);
        assert!(!error_str.is_empty(), "Error should have meaningful display message");
    }
}

// Integration test without session-core dependency
#[tokio::test]
async fn test_audio_device_manager_without_session_core() {
    let manager = AudioDeviceManager::new().await.unwrap();
    
    // Test that operations work without session-core client
    let call_id = CallId::new_v4();
    let device = manager.get_default_device(AudioDirection::Input).await.unwrap();
    
    // This should work but won't have session-core integration
    manager.start_capture(&call_id, device).await.unwrap();
    
    assert!(manager.is_capture_active(&call_id).await);
    
    manager.stop_capture(&call_id).await.unwrap();
    
    assert!(!manager.is_capture_active(&call_id).await);
} 