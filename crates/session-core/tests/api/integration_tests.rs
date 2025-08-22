//! Integration tests for UAC and UAS interaction

use rvoip_session_core::api::uac::{SimpleUacClient, UacBuilder};
use rvoip_session_core::api::uas::{SimpleUasServer, UasBuilder};
use rvoip_session_core::api::common::dtmf::{DtmfTone, send_dtmf_sequence};
use std::time::Duration;
use serial_test::serial;

#[tokio::test]
async fn test_dtmf_utilities() {
    // Test DTMF tone conversions
    assert_eq!(DtmfTone::Digit1.to_char(), '1');
    assert_eq!(DtmfTone::Star.to_char(), '*');
    assert_eq!(DtmfTone::Pound.to_char(), '#');
    assert_eq!(DtmfTone::A.to_char(), 'A');
    
    // Test from_char conversions
    assert_eq!(DtmfTone::from_char('0'), Some(DtmfTone::Digit0));
    assert_eq!(DtmfTone::from_char('*'), Some(DtmfTone::Star));
    assert_eq!(DtmfTone::from_char('#'), Some(DtmfTone::Pound));
    assert_eq!(DtmfTone::from_char('X'), None);
    
    // Test case insensitive for letters
    assert_eq!(DtmfTone::from_char('a'), Some(DtmfTone::A));
    assert_eq!(DtmfTone::from_char('A'), Some(DtmfTone::A));
}

#[tokio::test]
#[serial]
async fn test_parallel_server_creation() {
    // Test that we can create multiple servers on different ports
    let server1_future = SimpleUasServer::always_accept("127.0.0.1:15070");
    let server2_future = SimpleUasServer::always_reject(
        "127.0.0.1:15071",
        "Busy".to_string()
    );
    let server3_future = SimpleUasServer::always_forward(
        "127.0.0.1:15072",
        "sip:backup@example.com".to_string()
    );
    
    // Create all servers in parallel
    let (result1, result2, result3) = tokio::join!(
        server1_future,
        server2_future,
        server3_future
    );
    
    assert!(result1.is_ok(), "Server 1 failed: {:?}", result1.err());
    assert!(result2.is_ok(), "Server 2 failed: {:?}", result2.err());
    assert!(result3.is_ok(), "Server 3 failed: {:?}", result3.err());
    
    let server1 = result1.unwrap();
    let server2 = result2.unwrap();
    let server3 = result3.unwrap();
    
    // All servers should start with 0 active calls
    assert_eq!(server1.active_calls().await.unwrap(), 0);
    assert_eq!(server2.active_calls().await.unwrap(), 0);
    assert_eq!(server3.active_calls().await.unwrap(), 0);
    
    // Clean shutdown all servers
    let _ = tokio::join!(
        server1.shutdown(),
        server2.shutdown(),
        server3.shutdown()
    );
}

#[tokio::test]
#[serial]
async fn test_uac_uas_config_independence() {
    // Test that UAC and UAS can be configured independently
    
    // Create a UAC client
    let uac = UacBuilder::new("sip:client@example.com")
        .server("192.168.1.100:5060")
        .local_addr("0.0.0.0:5080")
        .user_agent("MyUAC/1.0")
        .call_timeout(120)
        .build()
        .await;
    
    assert!(uac.is_ok(), "UAC creation failed: {:?}", uac.err());
    
    // Create a UAS server on a different port
    let uas = UasBuilder::new("0.0.0.0:5090")
        .identity("sip:server@example.com")
        .user_agent("MyUAS/1.0")
        .max_concurrent_calls(50)
        .call_timeout(180)
        .build()
        .await;
    
    assert!(uas.is_ok(), "UAS creation failed: {:?}", uas.err());
    
    let uac_client = uac.unwrap();
    let uas_server = uas.unwrap();
    
    // Verify configurations are independent
    assert_eq!(uac_client.config().user_agent, "MyUAC/1.0");
    assert_eq!(uac_client.config().call_timeout, 120);
    
    // Note: We can't directly access UAS config but we know it's configured
    assert_eq!(uas_server.pending_count().await, 0);
    
    // Cleanup
    let _ = uac_client.shutdown().await;
    let _ = uas_server.shutdown().await;
}

#[tokio::test]
async fn test_call_state_transitions() {
    use rvoip_session_core::api::types::CallState;
    
    // Test that CallState variants work as expected
    let states = vec![
        CallState::Initiating,
        CallState::Ringing,
        CallState::Active,
        CallState::OnHold,
        CallState::Transferring,
        CallState::Terminating,
        CallState::Terminated,
        CallState::Cancelled,
        CallState::Failed("Network error".to_string()),
    ];
    
    // Verify all states are distinct
    for (i, state1) in states.iter().enumerate() {
        for (j, state2) in states.iter().enumerate() {
            if i == j {
                // Same state should be equal to itself
                assert_eq!(state1, state2);
            } else if !matches!((state1, state2), 
                              (CallState::Failed(_), CallState::Failed(_))) {
                // Different states should not be equal
                // (except Failed states with different messages)
                assert_ne!(state1, state2);
            }
        }
    }
}

#[tokio::test]
async fn test_audio_frame_types() {
    use rvoip_session_core::api::types::{AudioFrame, AudioStreamConfig};
    
    // Test AudioFrame creation
    let frame = AudioFrame::new(
        vec![0i16; 160], // 20ms of 8kHz audio
        8000,
        1,
        12345
    );
    
    assert_eq!(frame.samples.len(), 160);
    assert_eq!(frame.sample_rate, 8000);
    assert_eq!(frame.channels, 1);
    assert_eq!(frame.timestamp, 12345);
    
    // Test AudioStreamConfig
    let config = AudioStreamConfig {
        sample_rate: 48000,
        channels: 2,
        codec: "Opus".to_string(),
        frame_size_ms: 20,
        enable_aec: true,
        enable_agc: true,
        enable_vad: false,
    };
    
    assert_eq!(config.sample_rate, 48000);
    assert_eq!(config.codec, "Opus");
    assert!(config.enable_aec);
    assert!(config.enable_agc);
    assert!(!config.enable_vad);
}

#[tokio::test]
async fn test_error_handling() {
    use rvoip_session_core::SessionError;
    
    // Test that our error types work correctly
    let errors = vec![
        SessionError::InvalidState("Wrong state".to_string()),
        SessionError::SessionNotFound("123".to_string()),
        SessionError::MediaError("No codec".to_string()),
        SessionError::ConfigError("Invalid config".to_string()),
    ];
    
    for error in errors {
        // Verify error can be converted to string
        let error_str = error.to_string();
        assert!(!error_str.is_empty());
        
        // Verify error implements Debug
        let debug_str = format!("{:?}", error);
        assert!(!debug_str.is_empty());
    }
}