//! Tests for CANCEL Dialog Integration
//!
//! Tests the session-core functionality for CANCEL requests,
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

/// Test handler for CANCEL testing
#[derive(Debug)]
struct CancelTestHandler {
    cancelled_calls: Arc<tokio::sync::Mutex<Vec<SessionId>>>,
}

impl CancelTestHandler {
    fn new() -> Self {
        Self {
            cancelled_calls: Arc::new(tokio::sync::Mutex::new(Vec::new())),
        }
    }

    async fn get_cancelled_calls(&self) -> Vec<SessionId> {
        self.cancelled_calls.lock().await.clone()
    }
}

#[async_trait::async_trait]
impl CallHandler for CancelTestHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        // Accept calls to test cancellation
        CallDecision::Accept
    }

    async fn on_call_ended(&self, call: CallSession, reason: &str) {
        if reason.contains("cancel") || reason.contains("CANCEL") {
            self.cancelled_calls.lock().await.push(call.id().clone());
        }
        tracing::info!("Call {} ended with reason: {}", call.id(), reason);
    }
}

/// Create a test session manager for CANCEL testing
async fn create_cancel_test_manager() -> Result<Arc<SessionManager>, SessionError> {
    let handler = Arc::new(CancelTestHandler::new());
    
    SessionManagerBuilder::new()
        .with_sip_bind_address("127.0.0.1")
        .with_sip_port(0) // Use any available port
        .with_from_uri("sip:test@localhost")
        .with_handler(handler)
        .build()
        .await
}

#[tokio::test]
async fn test_outgoing_call_creation_for_cancel() {
    let manager = create_cancel_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create an outgoing call that could be cancelled
    let result = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com", 
        Some("v=0\r\no=alice 123 456 IN IP4 192.168.1.100\r\n...".to_string())
    ).await;
    
    assert!(result.is_ok());
    let call = result.unwrap();
    assert_eq!(call.state(), &CallState::Initiating);
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_call_termination_early_cancel() {
    let manager = create_cancel_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create an outgoing call
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("SDP offer".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Immediately terminate (simulates CANCEL scenario)
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
async fn test_multiple_early_cancellations() {
    let manager = create_cancel_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create multiple calls and cancel them quickly
    let mut calls = Vec::new();
    
    for i in 0..3 {
        let call = manager.create_outgoing_call(
            &format!("sip:caller{}@example.com", i),
            &format!("sip:target{}@example.com", i),
            Some(format!("SDP offer for call {}", i))
        ).await.unwrap();
        calls.push(call);
    }
    
    // Cancel all calls immediately
    for call in &calls {
        let result = manager.terminate_session(call.id()).await;
        assert!(result.is_ok());
    }
    
    // Wait for cleanup
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    // Verify all sessions are cleaned up
    let final_stats = manager.get_stats().await.unwrap();
    assert_eq!(final_stats.active_sessions, 0);
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_cancel_nonexistent_session() {
    let manager = create_cancel_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Try to cancel a non-existent session
    let fake_session_id = SessionId::new();
    let terminate_result = manager.terminate_session(&fake_session_id).await;
    assert!(terminate_result.is_err());
    assert!(matches!(terminate_result.unwrap_err(), SessionError::SessionNotFound(_)));
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_cancel_timing_scenarios() {
    let manager = create_cancel_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create call
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("SDP offer".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Wait a very short time before cancelling (simulates early CANCEL)
    tokio::time::sleep(Duration::from_millis(10)).await;
    
    let terminate_result = manager.terminate_session(&session_id).await;
    assert!(terminate_result.is_ok());
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_cancel_after_provisional_response() {
    let manager = create_cancel_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create call
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("SDP offer".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Wait longer before cancelling (simulates CANCEL after 18x response)
    tokio::time::sleep(Duration::from_millis(50)).await;
    
    let terminate_result = manager.terminate_session(&session_id).await;
    assert!(terminate_result.is_ok());
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_cancel_session_state_management() {
    let manager = create_cancel_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create call
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("SDP offer".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Verify session exists before cancel
    let session_before = manager.find_session(&session_id).await.unwrap();
    assert!(session_before.is_some());
    assert_eq!(session_before.unwrap().state(), &CallState::Initiating);
    
    // Cancel the session
    let terminate_result = manager.terminate_session(&session_id).await;
    assert!(terminate_result.is_ok());
    
    // Wait for state update
    tokio::time::sleep(Duration::from_millis(50)).await;
    
    // Verify session is removed
    let session_after = manager.find_session(&session_id).await.unwrap();
    assert!(session_after.is_none());
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_cancel_statistics_tracking() {
    let manager = create_cancel_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Check initial stats
    let initial_stats = manager.get_stats().await.unwrap();
    let initial_count = initial_stats.active_sessions;
    
    // Create call
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("SDP offer".to_string())
    ).await.unwrap();
    
    // Verify stats increased
    let mid_stats = manager.get_stats().await.unwrap();
    assert_eq!(mid_stats.active_sessions, initial_count + 1);
    
    // Cancel call
    let terminate_result = manager.terminate_session(call.id()).await;
    assert!(terminate_result.is_ok());
    
    // Wait for cleanup
    tokio::time::sleep(Duration::from_millis(50)).await;
    
    // Verify stats decreased
    let final_stats = manager.get_stats().await.unwrap();
    assert_eq!(final_stats.active_sessions, initial_count);
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_concurrent_cancellations() {
    let manager = create_cancel_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create multiple calls
    let mut calls = Vec::new();
    for i in 0..5 {
        let call = manager.create_outgoing_call(
            &format!("sip:caller{}@example.com", i),
            &format!("sip:target{}@example.com", i),
            Some(format!("SDP offer {}", i))
        ).await.unwrap();
        calls.push(call);
    }
    
    // Cancel all calls concurrently
    let mut cancel_tasks = Vec::new();
    for call in calls {
        let manager_clone = manager.clone();
        let session_id = call.id().clone();
        let task = tokio::spawn(async move {
            manager_clone.terminate_session(&session_id).await
        });
        cancel_tasks.push(task);
    }
    
    // Wait for all cancellations to complete
    for task in cancel_tasks {
        let result = task.await.unwrap();
        assert!(result.is_ok());
    }
    
    // Wait for cleanup
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    // Verify all sessions are cleaned up
    let final_stats = manager.get_stats().await.unwrap();
    assert_eq!(final_stats.active_sessions, 0);
    
    manager.stop().await.unwrap();
} 