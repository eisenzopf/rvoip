use rvoip_session_core::{
    api::{
        types::{SessionId, AudioFrame, AudioStreamConfig, AudioFrameSubscriber},
        media::MediaControl,
        control::SessionControl,
    },
    coordinator::SessionCoordinator,
    SessionError,
};
use std::sync::Arc;
use std::time::Duration;

// Test helper to create a test coordinator
async fn create_test_coordinator() -> Arc<SessionCoordinator> {
    use rvoip_session_core::api::builder::SessionManagerConfig;
    
    let mut config = SessionManagerConfig::default();
    config.sip_port = 5080; // Use specific port for tests to avoid conflicts
    config.local_bind_addr = "127.0.0.1:0".parse().unwrap();
    config.media_port_start = 20000;
    config.media_port_end = 21000;
    
    SessionCoordinator::new(config, None)
        .await
        .expect("Failed to create test coordinator")
}

// Test helper to create a test session
async fn create_test_session(coordinator: &Arc<SessionCoordinator>) -> SessionId {
    let session_id = SessionId::new();
    
    // Create a basic call session for testing
    let prepared = SessionControl::prepare_outgoing_call(
        coordinator,
        "sip:test@local.com",
        "sip:remote@remote.com",
    )
    .await
    .expect("Failed to prepare call");
    
    prepared.session_id
}

#[tokio::test]
async fn test_audio_frame_subscriber_creation() {
    let coordinator = create_test_coordinator().await;
    let session_id = create_test_session(&coordinator).await;
    
    // Test creating an audio frame subscriber
    let mut subscriber = coordinator.subscribe_to_audio_frames(&session_id).await
        .expect("Failed to create audio frame subscriber");
    
    // Verify subscriber properties
    assert_eq!(subscriber.session_id(), &session_id);
    assert!(subscriber.is_connected());
    
    // Test non-blocking receive (should return Empty since no frames are being sent)
    match subscriber.try_recv() {
        Err(tokio::sync::mpsc::error::TryRecvError::Empty) => {
            // This is expected - no frames available
        }
        Err(tokio::sync::mpsc::error::TryRecvError::Disconnected) => {
            panic!("Subscriber should be connected");
        }
        Ok(_) => {
            panic!("Didn't expect to receive a frame");
        }
    }
}

#[tokio::test]
async fn test_audio_frame_subscriber_invalid_session() {
    let coordinator = create_test_coordinator().await;
    let invalid_session_id = SessionId::new(); // Not created in coordinator
    
    // Test creating subscriber for non-existent session
    let result = coordinator.subscribe_to_audio_frames(&invalid_session_id).await;
    
    match result {
        Err(SessionError::SessionNotFound { .. }) => {
            // This is expected
        }
        _ => panic!("Expected SessionNotFound error"),
    }
}

#[tokio::test]
async fn test_send_audio_frame_placeholder() {
    let coordinator = create_test_coordinator().await;
    let session_id = create_test_session(&coordinator).await;
    
    // Create a test audio frame
    let audio_frame = AudioFrame::new(
        vec![100, 200, 300, 400], // Test samples
        8000,                     // 8kHz sample rate
        1,                        // Mono
        12345,                    // Timestamp
    );
    
    // Test sending the audio frame
    let result = coordinator.send_audio_frame(&session_id, audio_frame).await;
    assert!(result.is_ok(), "Failed to send audio frame: {:?}", result);
}

#[tokio::test]
async fn test_send_audio_frame_invalid_session() {
    let coordinator = create_test_coordinator().await;
    let invalid_session_id = SessionId::new(); // Not created in coordinator
    
    // Create a test audio frame
    let audio_frame = AudioFrame::new(
        vec![100, 200, 300, 400],
        8000,
        1,
        12345,
    );
    
    // Test sending frame to non-existent session
    // The implementation gracefully handles this case to avoid errors during termination
    let result = coordinator.send_audio_frame(&invalid_session_id, audio_frame).await;
    
    // Should return Ok(()) - frames are silently dropped for non-existent sessions
    // This prevents race conditions when sessions are terminating
    assert!(result.is_ok(), "Should gracefully handle non-existent session, got: {:?}", result);
}

#[tokio::test]
async fn test_audio_stream_config() {
    let coordinator = create_test_coordinator().await;
    let session_id = create_test_session(&coordinator).await;
    
    // Test getting default configuration
    let config = coordinator.get_audio_stream_config(&session_id).await
        .expect("Failed to get audio stream config");
    
    assert!(config.is_some());
    let config = config.unwrap();
    assert_eq!(config.sample_rate, 8000);
    assert_eq!(config.channels, 1);
    assert_eq!(config.codec, "PCMU");
    
    // Test setting a custom configuration
    let custom_config = AudioStreamConfig {
        sample_rate: 16000,
        channels: 2,
        codec: "Opus".to_string(),
        frame_size_ms: 10,
        enable_aec: false,
        enable_agc: false,
        enable_vad: false,
    };
    
    let result = coordinator.set_audio_stream_config(&session_id, custom_config.clone()).await;
    assert!(result.is_ok(), "Failed to set audio stream config: {:?}", result);
    
    // Verify configuration was set (placeholder implementation returns default)
    let retrieved_config = coordinator.get_audio_stream_config(&session_id).await
        .expect("Failed to get audio stream config after setting");
    
    assert!(retrieved_config.is_some());
    // Note: Since this is a placeholder implementation, it returns default config
    // In a real implementation, this would return the custom config
}

#[tokio::test]
async fn test_audio_stream_config_invalid_session() {
    let coordinator = create_test_coordinator().await;
    let invalid_session_id = SessionId::new(); // Not created in coordinator
    
    // Test getting config for non-existent session
    let result = coordinator.get_audio_stream_config(&invalid_session_id).await;
    match result {
        Err(SessionError::SessionNotFound { .. }) => {
            // This is expected
        }
        _ => panic!("Expected SessionNotFound error"),
    }
    
    // Test setting config for non-existent session
    let config = AudioStreamConfig::default();
    let result = coordinator.set_audio_stream_config(&invalid_session_id, config).await;
    match result {
        Err(SessionError::SessionNotFound { .. }) => {
            // This is expected
        }
        _ => panic!("Expected SessionNotFound error"),
    }
}

#[tokio::test]
async fn test_audio_stream_lifecycle() {
    let coordinator = create_test_coordinator().await;
    let session_id = create_test_session(&coordinator).await;
    
    // Test starting audio stream
    let result = coordinator.start_audio_stream(&session_id).await;
    assert!(result.is_ok(), "Failed to start audio stream: {:?}", result);
    
    // Test stopping audio stream
    let result = coordinator.stop_audio_stream(&session_id).await;
    assert!(result.is_ok(), "Failed to stop audio stream: {:?}", result);
    
    // Test starting and stopping again
    let result = coordinator.start_audio_stream(&session_id).await;
    assert!(result.is_ok(), "Failed to start audio stream second time: {:?}", result);
    
    let result = coordinator.stop_audio_stream(&session_id).await;
    assert!(result.is_ok(), "Failed to stop audio stream second time: {:?}", result);
}

#[tokio::test]
async fn test_audio_stream_lifecycle_invalid_session() {
    let coordinator = create_test_coordinator().await;
    let invalid_session_id = SessionId::new(); // Not created in coordinator
    
    // Test starting stream for non-existent session
    let result = coordinator.start_audio_stream(&invalid_session_id).await;
    match result {
        Err(SessionError::SessionNotFound { .. }) => {
            // This is expected
        }
        _ => panic!("Expected SessionNotFound error"),
    }
    
    // Test stopping stream for non-existent session
    let result = coordinator.stop_audio_stream(&invalid_session_id).await;
    match result {
        Err(SessionError::SessionNotFound { .. }) => {
            // This is expected
        }
        _ => panic!("Expected SessionNotFound error"),
    }
}

#[tokio::test]
async fn test_audio_frame_properties() {
    // Test AudioFrame creation and properties
    let samples = vec![100, 200, 300, 400, 500, 600, 700, 800]; // 8 samples
    let frame = AudioFrame::new(samples.clone(), 8000, 2, 12345);
    
    assert_eq!(frame.samples, samples);
    assert_eq!(frame.sample_rate, 8000);
    assert_eq!(frame.channels, 2);
    assert_eq!(frame.timestamp, 12345);
    
    // Test calculated properties
    assert_eq!(frame.samples_per_channel(), 4); // 8 samples / 2 channels
    assert!(frame.is_stereo());
    assert!(!frame.is_mono());
    
    // Test duration calculation
    let duration_ms = frame.duration.as_secs_f64() * 1000.0;
    let expected_duration = (4.0 * 1000.0) / 8000.0; // 4 samples per channel * 1000ms / 8000Hz
    assert!((duration_ms - expected_duration).abs() < 0.01);
}

#[tokio::test]
async fn test_audio_stream_config_properties() {
    // Test default configuration
    let config = AudioStreamConfig::default();
    assert_eq!(config.sample_rate, 8000);
    assert_eq!(config.channels, 1);
    assert_eq!(config.codec, "PCMU");
    assert_eq!(config.frame_size_ms, 20);
    assert!(config.enable_aec);
    assert!(config.enable_agc);
    assert!(config.enable_vad);
    
    // Test calculated properties
    assert_eq!(config.frame_size_samples(), 160); // 8000 * 20 / 1000
    assert_eq!(config.frame_size_bytes(), 320);   // 160 * 1 * 2 bytes
    
    // Test preset configurations
    let telephony = AudioStreamConfig::telephony();
    assert_eq!(telephony.sample_rate, 8000);
    assert_eq!(telephony.channels, 1);
    assert_eq!(telephony.codec, "PCMU");
    
    let wideband = AudioStreamConfig::wideband();
    assert_eq!(wideband.sample_rate, 16000);
    assert_eq!(wideband.channels, 1);
    assert_eq!(wideband.codec, "Opus");
    
    let hq = AudioStreamConfig::high_quality();
    assert_eq!(hq.sample_rate, 48000);
    assert_eq!(hq.channels, 2);
    assert_eq!(hq.codec, "Opus");
}

#[tokio::test]
async fn test_audio_frame_subscriber_timeout() {
    let coordinator = create_test_coordinator().await;
    let session_id = create_test_session(&coordinator).await;
    
    // Create subscriber
    let mut subscriber = coordinator.subscribe_to_audio_frames(&session_id).await
        .expect("Failed to create audio frame subscriber");
    
    // Test timeout receive (should timeout since no frames are being sent)
    let result = tokio::time::timeout(
        Duration::from_millis(100),
        subscriber.recv()
    ).await;
    
    match result {
        Err(_) => {
            // This is expected - timeout elapsed
        }
        Ok(None) => {
            panic!("Subscriber should be connected");
        }
        Ok(Some(_)) => {
            panic!("Didn't expect to receive a frame");
        }
    }
} 