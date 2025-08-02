//! Unit tests for RTP bridge callback functionality
//!
//! Tests the new RTP event callback system that bridges RTP packets
//! to external subscribers like session-core.

use std::sync::{Arc, Mutex};
use std::time::Instant;
use std::collections::HashMap;
use bytes::Bytes;
use tokio::sync::mpsc;

use rvoip_media_core::integration::{RtpBridge, RtpBridgeConfig, RtpEvent, RtpEventCallback};
use rvoip_media_core::integration::events::RtpParameters;
use rvoip_media_core::types::{MediaSessionId, MediaPacket};
use rvoip_media_core::codec::mapping::CodecMapper;
use rvoip_media_core::relay::controller::codec_detection::CodecDetector;
use rvoip_media_core::relay::controller::codec_fallback::CodecFallbackManager;
use rvoip_media_core::integration::IntegrationEvent;

#[tokio::test]
async fn test_rtp_bridge_callback_registration() {
    // Test that RTP event callbacks can be registered and cleared
    println!("🧪 Testing RTP bridge callback registration");
    
    let bridge = create_test_rtp_bridge().await;
    
    // Create a test callback
    let received_events = Arc::new(Mutex::new(Vec::new()));
    let received_events_clone = received_events.clone();
    
    let callback: RtpEventCallback = Arc::new(move |session_id, event| {
        let mut events = received_events_clone.lock().unwrap();
        events.push((session_id, event));
    });
    
    // Add the callback
    bridge.add_rtp_event_callback(callback).await;
    
    // Clear callbacks
    bridge.clear_rtp_event_callbacks().await;
    
    println!("✅ RTP bridge callback registration test PASSED!");
}

#[tokio::test]
async fn test_rtp_bridge_event_forwarding() {
    // Test that RTP packets are forwarded as events to registered callbacks
    println!("🧪 Testing RTP bridge event forwarding");
    
    let bridge = create_test_rtp_bridge().await;
    let session_id = MediaSessionId::new("test-session-1");
    
    // Register session with RTP parameters
    let rtp_params = RtpParameters {
        local_port: 5004,
        remote_address: "127.0.0.1".to_string(),
        remote_port: 5006,
        payload_type: 0, // PCMU
        ssrc: 0x12345678,
    };
    bridge.register_session(session_id.clone(), rtp_params).await
        .expect("Failed to register session");
    
    // Create callback to capture events
    let received_events = Arc::new(Mutex::new(Vec::new()));
    let received_events_clone = received_events.clone();
    
    let callback: RtpEventCallback = Arc::new(move |sess_id, event| {
        let mut events = received_events_clone.lock().unwrap();
        let session_str = sess_id.to_string();
        events.push((sess_id, event));
        println!("📥 Received RTP event for session: {}", session_str);
    });
    
    // Register callback
    bridge.add_rtp_event_callback(callback).await;
    
    // Create test packet
    let test_packet = MediaPacket {
        payload: Bytes::from(vec![0xFF, 0x7F, 0x00, 0x80]), // Test G.711 μ-law data
        payload_type: 0, // PCMU
        timestamp: 12345,
        sequence_number: 1,
        ssrc: 0x12345678,
        received_at: Instant::now(),
    };
    
    // Process the packet through the bridge
    bridge.process_incoming_packet(&session_id, test_packet.clone()).await
        .expect("Failed to process incoming packet");
    
    // Give some time for async processing
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    
    // Verify callback received the event
    let events = received_events.lock().unwrap();
    assert_eq!(events.len(), 1, "Should receive exactly one RTP event");
    
    let (received_session_id, received_event) = &events[0];
    assert_eq!(received_session_id.as_str(), session_id.as_str(), "Session ID should match");
    
    match received_event {
        RtpEvent::MediaReceived { payload_type, payload, timestamp, sequence_number, ssrc } => {
            assert_eq!(*payload_type, test_packet.payload_type, "Payload type should match");
            assert_eq!(payload, &test_packet.payload.to_vec(), "Payload should match");
            assert_eq!(*timestamp, test_packet.timestamp, "Timestamp should match");
            assert_eq!(*sequence_number, test_packet.sequence_number, "Sequence number should match");
            assert_eq!(*ssrc, test_packet.ssrc, "SSRC should match");
        }
        _ => panic!("Expected MediaReceived event"),
    }
    
    println!("✅ RTP bridge event forwarding test PASSED!");
}

#[tokio::test]
async fn test_multiple_callbacks() {
    // Test that multiple callbacks can be registered and all receive events
    println!("🧪 Testing multiple RTP callbacks");
    
    let bridge = create_test_rtp_bridge().await;
    let session_id = MediaSessionId::new("test-session-multi");
    
    // Register session with RTP parameters
    let rtp_params = RtpParameters {
        local_port: 5004,
        remote_address: "127.0.0.1".to_string(),
        remote_port: 5006,
        payload_type: 0, // PCMU
        ssrc: 0x12345678,
    };
    bridge.register_session(session_id.clone(), rtp_params).await
        .expect("Failed to register session");
    
    // Create multiple callbacks
    let received_events_1 = Arc::new(Mutex::new(Vec::new()));
    let received_events_2 = Arc::new(Mutex::new(Vec::new()));
    let received_events_3 = Arc::new(Mutex::new(Vec::new()));
    
    let callback_1 = {
        let events = received_events_1.clone();
        Arc::new(move |sess_id: MediaSessionId, event: RtpEvent| {
            let mut events = events.lock().unwrap();
            events.push((sess_id, event));
        })
    };
    
    let callback_2 = {
        let events = received_events_2.clone();
        Arc::new(move |sess_id: MediaSessionId, event: RtpEvent| {
            let mut events = events.lock().unwrap();
            events.push((sess_id, event));
        })
    };
    
    let callback_3 = {
        let events = received_events_3.clone();
        Arc::new(move |sess_id: MediaSessionId, event: RtpEvent| {
            let mut events = events.lock().unwrap();
            events.push((sess_id, event));
        })
    };
    
    // Register all callbacks
    bridge.add_rtp_event_callback(callback_1).await;
    bridge.add_rtp_event_callback(callback_2).await;
    bridge.add_rtp_event_callback(callback_3).await;
    
    // Send a test packet
    let test_packet = MediaPacket {
        payload: Bytes::from(vec![0xD5; 160]), // A-law silence
        payload_type: 8, // PCMA
        timestamp: 54321,
        sequence_number: 2,
        ssrc: 0x87654321,
        received_at: Instant::now(),
    };
    
    bridge.process_incoming_packet(&session_id, test_packet).await
        .expect("Failed to process packet");
    
    // Give time for processing
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    
    // Verify all callbacks received the event
    assert_eq!(received_events_1.lock().unwrap().len(), 1, "Callback 1 should receive event");
    assert_eq!(received_events_2.lock().unwrap().len(), 1, "Callback 2 should receive event");
    assert_eq!(received_events_3.lock().unwrap().len(), 1, "Callback 3 should receive event");
    
    println!("✅ Multiple RTP callbacks test PASSED!");
}

#[tokio::test]
async fn test_callback_isolation_between_sessions() {
    // Test that callbacks receive events only for packets from all sessions
    // (RTP events are broadcasted to all callbacks, session filtering happens at callback level)
    println!("🧪 Testing callback isolation between sessions");
    
    let bridge = create_test_rtp_bridge().await;
    let session_1 = MediaSessionId::new("session-1");
    let session_2 = MediaSessionId::new("session-2");
    
    // Register both sessions
    let rtp_params = RtpParameters {
        local_port: 5004,
        remote_address: "127.0.0.1".to_string(),
        remote_port: 5006,
        payload_type: 0,
        ssrc: 0x12345678,
    };
    bridge.register_session(session_1.clone(), rtp_params).await.unwrap();
    let rtp_params_2 = RtpParameters {
        local_port: 5008,
        remote_address: "127.0.0.1".to_string(),
        remote_port: 5010,
        payload_type: 8,
        ssrc: 0x87654321,
    };
    bridge.register_session(session_2.clone(), rtp_params_2).await.unwrap();
    
    // Create callback that tracks which sessions it receives events for
    let received_sessions = Arc::new(Mutex::new(Vec::new()));
    let received_sessions_clone = received_sessions.clone();
    
    let callback: RtpEventCallback = Arc::new(move |sess_id, _event| {
        let mut sessions = received_sessions_clone.lock().unwrap();
        sessions.push(sess_id);
    });
    
    bridge.add_rtp_event_callback(callback).await;
    
    // Send packets to both sessions
    let packet_1 = MediaPacket {
        payload: Bytes::from(vec![0xFF; 80]),
        payload_type: 0,
        timestamp: 1000,
        sequence_number: 1,
        ssrc: 0x11111111,
        received_at: Instant::now(),
    };
    
    let packet_2 = MediaPacket {
        payload: Bytes::from(vec![0xD5; 160]),
        payload_type: 8,
        timestamp: 2000,
        sequence_number: 1,
        ssrc: 0x22222222,
        received_at: Instant::now(),
    };
    
    bridge.process_incoming_packet(&session_1, packet_1).await.unwrap();
    bridge.process_incoming_packet(&session_2, packet_2).await.unwrap();
    
    tokio::time::sleep(tokio::time::Duration::from_millis(50)).await;
    
    // Verify callback received events for both sessions
    let sessions = received_sessions.lock().unwrap();
    assert_eq!(sessions.len(), 2, "Should receive events for both sessions");
    
    // Verify both session IDs are present
    let session_ids: std::collections::HashSet<String> = sessions.iter()
        .map(|s| s.as_str().to_string())
        .collect();
    
    assert!(session_ids.contains(session_1.as_str()), "Should contain session 1");
    assert!(session_ids.contains(session_2.as_str()), "Should contain session 2");
    
    println!("✅ Callback session isolation test PASSED!");
}

#[tokio::test]
async fn test_no_callbacks_registered() {
    // Test that packet processing works even when no callbacks are registered
    println!("🧪 Testing packet processing without callbacks");
    
    let bridge = create_test_rtp_bridge().await;
    let session_id = MediaSessionId::new("no-callback-session");
    
    let rtp_params = RtpParameters {
        local_port: 5004,
        remote_address: "127.0.0.1".to_string(),
        remote_port: 5006,
        payload_type: 0,
        ssrc: 0x12345678,
    };
    bridge.register_session(session_id.clone(), rtp_params).await.unwrap();
    
    let test_packet = MediaPacket {
        payload: Bytes::from(vec![0xFF; 160]),
        payload_type: 0,
        timestamp: 99999,
        sequence_number: 99,
        ssrc: 0x99999999,
        received_at: Instant::now(),
    };
    
    // This should not fail even without callbacks
    bridge.process_incoming_packet(&session_id, test_packet).await
        .expect("Should process packet successfully even without callbacks");
    
    println!("✅ No callbacks registered test PASSED!");
}

/// Helper function to create a test RTP bridge
async fn create_test_rtp_bridge() -> Arc<RtpBridge> {
    let (integration_event_tx, _integration_event_rx) = mpsc::unbounded_channel::<IntegrationEvent>();
    let codec_mapper = Arc::new(CodecMapper::new());
    let codec_detector = Arc::new(CodecDetector::new(codec_mapper.clone()));
    let fallback_manager = Arc::new(CodecFallbackManager::new(codec_detector.clone(), codec_mapper.clone()));
    
    Arc::new(RtpBridge::new(
        RtpBridgeConfig::default(),
        integration_event_tx,
        codec_mapper,
        codec_detector,
        fallback_manager,
    ))
}