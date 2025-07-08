//! Integration tests for ClientManager audio device integration
//!
//! These tests verify that the ClientManager properly integrates with AudioDeviceManager
//! to provide audio device functionality.

use std::sync::Arc;
use uuid::Uuid;

use rvoip_client_core::{
    ClientManager, ClientConfig,
    audio::AudioDirection,
    call::CallId,
    client::config::MediaConfig,
};

/// Test that ClientManager can list audio devices
#[tokio::test]
async fn test_client_manager_list_audio_devices() {
    let config = ClientConfig {
        local_sip_addr: "127.0.0.1:5060".parse().unwrap(),
        local_media_addr: "127.0.0.1:7060".parse().unwrap(),
        user_agent: "Test Client".to_string(),
        media: MediaConfig::default(),
        max_concurrent_calls: 10,
        session_timeout_secs: 300,
        enable_audio: true,
        enable_video: false,
        domain: None,
    };

    let client_manager = ClientManager::new(config).await.unwrap();
    
    // Test listing input devices
    let input_devices = client_manager.list_audio_devices(AudioDirection::Input).await.unwrap();
    assert!(!input_devices.is_empty(), "Should have at least one input device");
    
    // Test listing output devices
    let output_devices = client_manager.list_audio_devices(AudioDirection::Output).await.unwrap();
    assert!(!output_devices.is_empty(), "Should have at least one output device");
    
    println!("✅ ClientManager can list audio devices");
}

/// Test that ClientManager can get default audio devices
#[tokio::test]
async fn test_client_manager_get_default_audio_devices() {
    let config = ClientConfig {
        local_sip_addr: "127.0.0.1:5061".parse().unwrap(),
        local_media_addr: "127.0.0.1:7061".parse().unwrap(),
        user_agent: "Test Client".to_string(),
        media: MediaConfig::default(),
        max_concurrent_calls: 10,
        session_timeout_secs: 300,
        enable_audio: true,
        enable_video: false,
        domain: None,
    };

    let client_manager = ClientManager::new(config).await.unwrap();
    
    // Test getting default input device
    let default_input = client_manager.get_default_audio_device(AudioDirection::Input).await.unwrap();
    assert_eq!(default_input.direction, AudioDirection::Input);
    assert!(!default_input.name.is_empty());
    
    // Test getting default output device
    let default_output = client_manager.get_default_audio_device(AudioDirection::Output).await.unwrap();
    assert_eq!(default_output.direction, AudioDirection::Output);
    assert!(!default_output.name.is_empty());
    
    println!("✅ ClientManager can get default audio devices");
}

/// Test that ClientManager audio session management works
#[tokio::test]
async fn test_client_manager_audio_session_management() {
    let config = ClientConfig {
        local_sip_addr: "127.0.0.1:5062".parse().unwrap(),
        local_media_addr: "127.0.0.1:7062".parse().unwrap(),
        user_agent: "Test Client".to_string(),
        media: MediaConfig::default(),
        max_concurrent_calls: 10,
        session_timeout_secs: 300,
        enable_audio: true,
        enable_video: false,
        domain: None,
    };

    let client_manager = ClientManager::new(config).await.unwrap();
    
    // Test getting active audio sessions (should be empty initially)
    let (playback_sessions, capture_sessions) = client_manager.get_active_audio_sessions().await;
    assert!(playback_sessions.is_empty(), "Should have no active playback sessions initially");
    assert!(capture_sessions.is_empty(), "Should have no active capture sessions initially");
    
    // Test checking if audio is active for a non-existent call
    let fake_call_id = Uuid::new_v4();
    let playback_active = client_manager.is_audio_playback_active(&fake_call_id).await;
    let capture_active = client_manager.is_audio_capture_active(&fake_call_id).await;
    
    assert!(!playback_active, "Playback should not be active for non-existent call");
    assert!(!capture_active, "Capture should not be active for non-existent call");
    
    println!("✅ ClientManager audio session management works");
}

/// Test that ClientManager audio operations fail gracefully for invalid calls
#[tokio::test]
async fn test_client_manager_audio_operations_error_handling() {
    let config = ClientConfig {
        local_sip_addr: "127.0.0.1:5063".parse().unwrap(),
        local_media_addr: "127.0.0.1:7063".parse().unwrap(),
        user_agent: "Test Client".to_string(),
        media: MediaConfig::default(),
        max_concurrent_calls: 10,
        session_timeout_secs: 300,
        enable_audio: true,
        enable_video: false,
        domain: None,
    };

    let client_manager = ClientManager::new(config).await.unwrap();
    let fake_call_id = Uuid::new_v4();
    
    // Test starting audio playback for non-existent call
    let result = client_manager.start_audio_playback(&fake_call_id, "mock-output-1").await;
    assert!(result.is_err(), "Should fail for non-existent call");
    
    // Test starting audio capture for non-existent call
    let result = client_manager.start_audio_capture(&fake_call_id, "mock-input-1").await;
    assert!(result.is_err(), "Should fail for non-existent call");
    
    // Test stopping audio operations for non-existent call (should still work)
    let result = client_manager.stop_audio_playback(&fake_call_id).await;
    assert!(result.is_ok(), "Stop operations should be safe for non-existent calls");
    
    let result = client_manager.stop_audio_capture(&fake_call_id).await;
    assert!(result.is_ok(), "Stop operations should be safe for non-existent calls");
    
    println!("✅ ClientManager audio operations error handling works");
}

/// Test that ClientManager can stop all audio sessions
#[tokio::test]
async fn test_client_manager_stop_all_audio_sessions() {
    let config = ClientConfig {
        local_sip_addr: "127.0.0.1:5064".parse().unwrap(),
        local_media_addr: "127.0.0.1:7064".parse().unwrap(),
        user_agent: "Test Client".to_string(),
        media: MediaConfig::default(),
        max_concurrent_calls: 10,
        session_timeout_secs: 300,
        enable_audio: true,
        enable_video: false,
        domain: None,
    };

    let client_manager = ClientManager::new(config).await.unwrap();
    
    // Test stopping all audio sessions (should work even if no sessions exist)
    let result = client_manager.stop_all_audio_sessions().await;
    assert!(result.is_ok(), "Should be able to stop all audio sessions");
    
    // Verify no sessions are active after stopping all
    let (playback_sessions, capture_sessions) = client_manager.get_active_audio_sessions().await;
    assert!(playback_sessions.is_empty(), "Should have no active playback sessions after stopping all");
    assert!(capture_sessions.is_empty(), "Should have no active capture sessions after stopping all");
    
    println!("✅ ClientManager can stop all audio sessions");
}

/// Test that ClientManager properly exposes audio device integration through its API
#[tokio::test]
async fn test_client_manager_audio_device_integration_api() {
    let config = ClientConfig {
        local_sip_addr: "127.0.0.1:5065".parse().unwrap(),
        local_media_addr: "127.0.0.1:7065".parse().unwrap(),
        user_agent: "Test Client".to_string(),
        media: MediaConfig::default(),
        max_concurrent_calls: 10,
        session_timeout_secs: 300,
        enable_audio: true,
        enable_video: false,
        domain: None,
    };

    let client_manager = ClientManager::new(config).await.unwrap();
    
    // Test that all audio device methods are available and working
    let input_devices = client_manager.list_audio_devices(AudioDirection::Input).await.unwrap();
    let output_devices = client_manager.list_audio_devices(AudioDirection::Output).await.unwrap();
    
    assert!(!input_devices.is_empty(), "Should have input devices");
    assert!(!output_devices.is_empty(), "Should have output devices");
    
    // Test that we can get default devices
    let default_input = client_manager.get_default_audio_device(AudioDirection::Input).await.unwrap();
    let default_output = client_manager.get_default_audio_device(AudioDirection::Output).await.unwrap();
    
    assert_eq!(default_input.direction, AudioDirection::Input);
    assert_eq!(default_output.direction, AudioDirection::Output);
    
    // Test that we can query session state
    let (playback_sessions, capture_sessions) = client_manager.get_active_audio_sessions().await;
    assert!(playback_sessions.is_empty());
    assert!(capture_sessions.is_empty());
    
    println!("✅ ClientManager audio device integration API works correctly");
} 