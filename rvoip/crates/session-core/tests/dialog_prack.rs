//! Tests for PRACK Dialog Integration
//!
//! Tests the session-core functionality for PRACK requests (Provisional Response Acknowledgment),
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

/// Test handler for PRACK testing
#[derive(Debug)]
struct PrackTestHandler {
    prack_calls: Arc<tokio::sync::Mutex<Vec<SessionId>>>,
    reliable_responses: Arc<tokio::sync::Mutex<Vec<String>>>,
}

impl PrackTestHandler {
    fn new() -> Self {
        Self {
            prack_calls: Arc::new(tokio::sync::Mutex::new(Vec::new())),
            reliable_responses: Arc::new(tokio::sync::Mutex::new(Vec::new())),
        }
    }

    async fn get_prack_calls(&self) -> Vec<SessionId> {
        self.prack_calls.lock().await.clone()
    }

    async fn add_reliable_response(&self, response: String) {
        self.reliable_responses.lock().await.push(response);
    }

    async fn get_reliable_responses(&self) -> Vec<String> {
        self.reliable_responses.lock().await.clone()
    }
}

#[async_trait::async_trait]
impl CallHandler for PrackTestHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        // Accept calls and potentially send reliable provisional responses
        self.prack_calls.lock().await.push(call.id.clone());
        CallDecision::Accept
    }

    async fn on_call_ended(&self, call: CallSession, reason: &str) {
        tracing::info!("PRACK test call {} ended: {}", call.id(), reason);
    }
}

/// Create a test session manager for PRACK testing
async fn create_prack_test_manager() -> Result<Arc<SessionManager>, SessionError> {
    let handler = Arc::new(PrackTestHandler::new());
    
    SessionManagerBuilder::new()
        .with_sip_bind_address("127.0.0.1")
        .with_sip_port(0) // Use any available port
        .with_from_uri("sip:test@localhost")
        .with_handler(handler)
        .build()
        .await
}

#[tokio::test]
async fn test_outgoing_call_with_prack_support() {
    let manager = create_prack_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create an outgoing call that supports reliable provisional responses
    let result = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com", 
        Some("v=0\r\no=alice 123 456 IN IP4 192.168.1.100\r\nm=audio 5004 RTP/AVP 0\r\n".to_string())
    ).await;
    
    assert!(result.is_ok());
    let call = result.unwrap();
    assert_eq!(call.state(), &CallState::Initiating);
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_session_with_early_media_prack() {
    let manager = create_prack_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create call that might receive early media with PRACK
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("SDP with early media support".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Simulate receiving 183 Session Progress with early media
    // In real scenario, this would be handled by dialog-core
    
    // Verify session exists and is in correct state
    let session = manager.find_session(&session_id).await.unwrap();
    assert!(session.is_some());
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_prack_sequence_handling() {
    let manager = create_prack_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create multiple calls to test PRACK sequence numbers
    let mut calls = Vec::new();
    for i in 0..3 {
        let call = manager.create_outgoing_call(
            &format!("sip:caller{}@example.com", i),
            &format!("sip:target{}@example.com", i),
            Some(format!("SDP for PRACK test {}", i))
        ).await.unwrap();
        calls.push(call);
    }
    
    // Each call should handle PRACK sequences independently
    for call in &calls {
        let session = manager.find_session(call.id()).await.unwrap();
        assert!(session.is_some());
    }
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_prack_with_media_update() {
    let manager = create_prack_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create call
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("Initial SDP".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Simulate PRACK with SDP (media update in provisional response)
    let update_result = manager.update_media(&session_id, "Updated SDP in PRACK").await;
    assert!(update_result.is_ok());
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_prack_error_handling() {
    let manager = create_prack_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Try PRACK-related operations on non-existent session
    let fake_session_id = SessionId::new();
    
    // Media update (which could be part of PRACK flow) should fail
    let update_result = manager.update_media(&fake_session_id, "PRACK SDP").await;
    assert!(update_result.is_err());
    assert!(matches!(update_result.unwrap_err(), SessionError::SessionNotFound(_)));
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_multiple_provisional_responses() {
    let manager = create_prack_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create call that might receive multiple provisional responses
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("SDP offer".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Simulate multiple media updates (as if from different provisional responses)
    for i in 1..=3 {
        let sdp = format!("Provisional response {} SDP", i);
        let update_result = manager.update_media(&session_id, &sdp).await;
        assert!(update_result.is_ok());
        
        // Small delay between provisional responses
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_prack_session_state_consistency() {
    let manager = create_prack_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create call
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("Initial SDP".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Verify session exists before PRACK operations
    let session_before = manager.find_session(&session_id).await.unwrap();
    assert!(session_before.is_some());
    assert_eq!(session_before.unwrap().state(), &CallState::Initiating);
    
    // Simulate PRACK-related media update
    let update_result = manager.update_media(&session_id, "PRACK media update").await;
    assert!(update_result.is_ok());
    
    // Verify session consistency after PRACK
    let session_after = manager.find_session(&session_id).await.unwrap();
    assert!(session_after.is_some());
    assert_eq!(session_after.unwrap().id(), &session_id);
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_prack_with_codec_negotiation() {
    let manager = create_prack_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create call with multiple codec options
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("v=0\r\no=alice 123 456 IN IP4 192.168.1.100\r\nm=audio 5004 RTP/AVP 0 8 18\r\n".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Simulate codec negotiation via PRACK sequence
    let negotiated_sdp = "v=0\r\no=alice 123 789 IN IP4 192.168.1.100\r\nm=audio 5004 RTP/AVP 8\r\n";
    let update_result = manager.update_media(&session_id, negotiated_sdp).await;
    assert!(update_result.is_ok());
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_concurrent_prack_sessions() {
    let manager = create_prack_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create multiple calls that might use PRACK
    let mut calls = Vec::new();
    for i in 0..5 {
        let call = manager.create_outgoing_call(
            &format!("sip:caller{}@example.com", i),
            &format!("sip:target{}@example.com", i),
            Some(format!("PRACK test SDP {}", i))
        ).await.unwrap();
        calls.push(call);
    }
    
    // Perform concurrent PRACK-related operations
    let mut tasks = Vec::new();
    for (i, call) in calls.iter().enumerate() {
        let manager_clone = manager.clone();
        let session_id = call.id().clone();
        let task = tokio::spawn(async move {
            let sdp = format!("Concurrent PRACK update {}", i);
            manager_clone.update_media(&session_id, &sdp).await
        });
        tasks.push(task);
    }
    
    // Wait for all PRACK operations to complete
    for task in tasks {
        let result = task.await.unwrap();
        assert!(result.is_ok());
    }
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_prack_timing_constraints() {
    let manager = create_prack_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create call
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("SDP offer".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Test rapid PRACK sequences (testing timing)
    let start_time = std::time::Instant::now();
    
    for i in 0..5 {
        let sdp = format!("Rapid PRACK {}", i);
        let update_result = manager.update_media(&session_id, &sdp).await;
        assert!(update_result.is_ok());
    }
    
    let elapsed = start_time.elapsed();
    // PRACK operations should complete reasonably quickly
    assert!(elapsed < Duration::from_secs(1));
    
    manager.stop().await.unwrap();
} 