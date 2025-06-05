//! Tests for INFO Dialog Integration
//!
//! Tests the session-core functionality for INFO requests (in-dialog information),
//! ensuring proper integration with the underlying dialog layer.

use std::sync::Arc;
use std::time::Duration;
use rvoip_session_core::{
    SessionManager,
    SessionError,
    api::{
        types::{CallState, SessionId, IncomingCall, CallSession, CallDecision},
        handlers::CallHandler,
        builder::SessionManagerBuilder,
    },
};

/// Test handler for INFO testing
#[derive(Debug)]
struct InfoTestHandler {
    info_messages: Arc<tokio::sync::Mutex<Vec<(SessionId, String)>>>,
    dtmf_events: Arc<tokio::sync::Mutex<Vec<(SessionId, String)>>>,
}

impl InfoTestHandler {
    fn new() -> Self {
        Self {
            info_messages: Arc::new(tokio::sync::Mutex::new(Vec::new())),
            dtmf_events: Arc::new(tokio::sync::Mutex::new(Vec::new())),
        }
    }

    async fn add_info_message(&self, session_id: SessionId, content: String) {
        self.info_messages.lock().await.push((session_id, content));
    }

    async fn add_dtmf_event(&self, session_id: SessionId, digits: String) {
        self.dtmf_events.lock().await.push((session_id, digits));
    }

    async fn get_info_messages(&self) -> Vec<(SessionId, String)> {
        self.info_messages.lock().await.clone()
    }

    async fn get_dtmf_events(&self) -> Vec<(SessionId, String)> {
        self.dtmf_events.lock().await.clone()
    }
}

#[async_trait::async_trait]
impl CallHandler for InfoTestHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        CallDecision::Accept
    }

    async fn on_call_ended(&self, call: CallSession, reason: &str) {
        tracing::info!("INFO test call {} ended: {}", call.id(), reason);
    }
}

/// Create a test session manager for INFO testing
async fn create_info_test_manager() -> Result<Arc<SessionManager>, SessionError> {
    let handler = Arc::new(InfoTestHandler::new());
    
    SessionManagerBuilder::new()
        .with_sip_bind_address("127.0.0.1")
        .with_sip_port(0) // Use any available port
        .with_from_uri("sip:test@localhost")
        .with_handler(handler)
        .build()
        .await
}

#[tokio::test]
async fn test_basic_dtmf_sending() {
    let manager = create_info_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create call
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com", 
        Some("v=0\r\no=alice 123 456 IN IP4 192.168.1.100\r\nm=audio 5004 RTP/AVP 0\r\n".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Send DTMF digits via INFO
    let dtmf_result = manager.send_dtmf(&session_id, "123").await;
    assert!(dtmf_result.is_ok());
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_dtmf_digit_sequences() {
    let manager = create_info_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create call
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("SDP offer".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Test various DTMF sequences
    let dtmf_sequences = vec![
        "0", "1", "2", "3", "4", "5", "6", "7", "8", "9",
        "*", "#", "A", "B", "C", "D",
        "123456789", "*#0", "1234*567#890"
    ];
    
    for sequence in dtmf_sequences {
        let dtmf_result = manager.send_dtmf(&session_id, sequence).await;
        assert!(dtmf_result.is_ok());
        
        // Small delay between sequences
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_dtmf_special_characters() {
    let manager = create_info_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create call
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("SDP offer".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Test special DTMF characters
    let special_dtmf = vec!["*", "#", "A", "B", "C", "D"];
    
    for digit in special_dtmf {
        let dtmf_result = manager.send_dtmf(&session_id, digit).await;
        assert!(dtmf_result.is_ok());
    }
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_rapid_dtmf_sending() {
    let manager = create_info_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create call
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("SDP offer".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Send rapid DTMF sequence
    let start_time = std::time::Instant::now();
    
    for i in 0..10 {
        let digit = format!("{}", i % 10);
        let dtmf_result = manager.send_dtmf(&session_id, &digit).await;
        assert!(dtmf_result.is_ok());
        // No delay - testing rapid sending
    }
    
    let elapsed = start_time.elapsed();
    // DTMF operations should complete quickly
    assert!(elapsed < Duration::from_secs(1));
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_dtmf_nonexistent_session() {
    let manager = create_info_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Try to send DTMF to non-existent session
    let fake_session_id = SessionId::new();
    let dtmf_result = manager.send_dtmf(&fake_session_id, "123").await;
    assert!(dtmf_result.is_err());
    assert!(matches!(dtmf_result.unwrap_err(), SessionError::SessionNotFound(_)));
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_dtmf_concurrent_sessions() {
    let manager = create_info_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create multiple calls
    let mut calls = Vec::new();
    for i in 0..5 {
        let call = manager.create_outgoing_call(
            &format!("sip:caller{}@example.com", i),
            &format!("sip:target{}@example.com", i),
            Some(format!("DTMF test SDP {}", i))
        ).await.unwrap();
        calls.push(call);
    }
    
    // Send DTMF to all calls concurrently
    let mut dtmf_tasks = Vec::new();
    for (i, call) in calls.iter().enumerate() {
        let manager_clone = manager.clone();
        let session_id = call.id().clone();
        let digits = format!("{}", i);
        let task = tokio::spawn(async move {
            manager_clone.send_dtmf(&session_id, &digits).await
        });
        dtmf_tasks.push(task);
    }
    
    // Wait for all DTMF operations to complete
    for task in dtmf_tasks {
        let result = task.await.unwrap();
        assert!(result.is_ok());
    }
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_dtmf_during_hold() {
    let manager = create_info_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create call
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("SDP offer".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Put call on hold
    let hold_result = manager.hold_session(&session_id).await;
    assert!(hold_result.is_ok());
    
    // Send DTMF while on hold
    let dtmf_result = manager.send_dtmf(&session_id, "123").await;
    assert!(dtmf_result.is_ok());
    
    // Resume call
    let resume_result = manager.resume_session(&session_id).await;
    assert!(resume_result.is_ok());
    
    // Send DTMF after resume
    let dtmf_result2 = manager.send_dtmf(&session_id, "456").await;
    assert!(dtmf_result2.is_ok());
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_dtmf_with_media_updates() {
    let manager = create_info_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create call
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("Initial SDP".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Send DTMF before media update
    let dtmf_result1 = manager.send_dtmf(&session_id, "123").await;
    assert!(dtmf_result1.is_ok());
    
    // Update media
    let update_result = manager.update_media(&session_id, "Updated SDP").await;
    assert!(update_result.is_ok());
    
    // Send DTMF after media update
    let dtmf_result2 = manager.send_dtmf(&session_id, "456").await;
    assert!(dtmf_result2.is_ok());
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_dtmf_empty_string() {
    let manager = create_info_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create call
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("SDP offer".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Try to send empty DTMF
    let dtmf_result = manager.send_dtmf(&session_id, "").await;
    // This should either succeed (empty INFO) or fail gracefully
    // The important thing is that it doesn't panic
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_dtmf_session_state_consistency() {
    let manager = create_info_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create call
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("SDP offer".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Verify session exists before DTMF
    let session_before = manager.find_session(&session_id).await.unwrap();
    assert!(session_before.is_some());
    
    // Send DTMF
    let dtmf_result = manager.send_dtmf(&session_id, "123").await;
    assert!(dtmf_result.is_ok());
    
    // Verify session still exists after DTMF
    let session_after = manager.find_session(&session_id).await.unwrap();
    assert!(session_after.is_some());
    assert_eq!(session_after.unwrap().id(), &session_id);
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_long_dtmf_sequences() {
    let manager = create_info_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create call
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("SDP offer".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Test very long DTMF sequence
    let long_sequence = "1234567890*#ABCD".repeat(10); // 160 characters
    let dtmf_result = manager.send_dtmf(&session_id, &long_sequence).await;
    assert!(dtmf_result.is_ok());
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_dtmf_timing_requirements() {
    let manager = create_info_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create call
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("SDP offer".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Test timing of individual DTMF operations
    for i in 0..5 {
        let start_time = std::time::Instant::now();
        let dtmf_result = manager.send_dtmf(&session_id, &format!("{}", i)).await;
        assert!(dtmf_result.is_ok());
        let duration = start_time.elapsed();
        
        // Each DTMF should complete quickly
        assert!(duration < Duration::from_millis(100));
        
        // Small delay between digits (realistic DTMF timing)
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_dtmf_after_transfer() {
    let manager = create_info_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create call
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("SDP offer".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Send DTMF before transfer
    let dtmf_result1 = manager.send_dtmf(&session_id, "123").await;
    assert!(dtmf_result1.is_ok());
    
    // Initiate transfer
    let transfer_result = manager.transfer_session(&session_id, "sip:charlie@example.com").await;
    assert!(transfer_result.is_ok());
    
    // Send DTMF after transfer initiation
    let dtmf_result2 = manager.send_dtmf(&session_id, "456").await;
    assert!(dtmf_result2.is_ok());
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_dtmf_before_termination() {
    let manager = create_info_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create call
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("SDP offer".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Send DTMF
    let dtmf_result = manager.send_dtmf(&session_id, "123").await;
    assert!(dtmf_result.is_ok());
    
    // Terminate session
    let terminate_result = manager.terminate_session(&session_id).await;
    assert!(terminate_result.is_ok());
    
    // Wait for cleanup
    tokio::time::sleep(Duration::from_millis(50)).await;
    
    // Verify session is removed
    let session_after = manager.find_session(&session_id).await.unwrap();
    assert!(session_after.is_none());
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_mixed_dtmf_operations() {
    let manager = create_info_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create multiple calls
    let mut calls = Vec::new();
    for i in 0..3 {
        let call = manager.create_outgoing_call(
            &format!("sip:caller{}@example.com", i),
            &format!("sip:target{}@example.com", i),
            Some(format!("Mixed DTMF test SDP {}", i))
        ).await.unwrap();
        calls.push(call);
    }
    
    // Perform mixed operations on different calls
    for (i, call) in calls.iter().enumerate() {
        let session_id = call.id();
        
        match i {
            0 => {
                // Call 0: Simple DTMF
                let _ = manager.send_dtmf(session_id, "123").await;
            },
            1 => {
                // Call 1: DTMF with hold/resume
                let _ = manager.hold_session(session_id).await;
                let _ = manager.send_dtmf(session_id, "456").await;
                let _ = manager.resume_session(session_id).await;
            },
            2 => {
                // Call 2: DTMF with media update
                let _ = manager.send_dtmf(session_id, "789").await;
                let _ = manager.update_media(session_id, "Updated SDP").await;
            },
            _ => {}
        }
    }
    
    // All operations should complete successfully
    let final_stats = manager.get_stats().await.unwrap();
    assert_eq!(final_stats.active_sessions, 3);
    
    manager.stop().await.unwrap();
} 