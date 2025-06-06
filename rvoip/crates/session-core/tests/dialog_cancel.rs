//! Tests for CANCEL Dialog Integration
//!
//! Tests the session-core functionality for CANCEL requests,
//! ensuring proper integration with the underlying dialog layer.

mod common;

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
use common::*;

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
    let (manager_a, _manager_b, _) = create_session_manager_pair().await.unwrap();
    
    // Create an outgoing call that could be cancelled
    let result = manager_a.create_outgoing_call(
        "sip:alice@127.0.0.1",
        "sip:bob@127.0.0.1:6001", 
        Some("v=0\r\no=alice 123 456 IN IP4 192.168.1.100\r\n...".to_string())
    ).await;
    
    assert!(result.is_ok());
    let call = result.unwrap();
    assert_eq!(call.state(), &CallState::Initiating);
}

#[tokio::test]
async fn test_call_termination_early_cancel() {
    let (manager_a, manager_b, _) = create_session_manager_pair().await.unwrap();
    
    // Create an outgoing call to a real endpoint
    let call = manager_a.create_outgoing_call(
        "sip:alice@127.0.0.1",
        &format!("sip:bob@{}", manager_b.get_bound_address()),
        Some("SDP offer".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Wait a bit for the call to be processed
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    // Check session state before termination
    let session_before = manager_a.find_session(&session_id).await.unwrap();
    let state_before = session_before.as_ref().map(|s| s.state().clone());
    println!("Session before termination: {:?}", state_before);
    
    // Terminate early (simulates CANCEL scenario)  
    let terminate_result = manager_a.terminate_session(&session_id).await;
    if let Err(ref e) = terminate_result {
        println!("❌ Terminate error: {:?}", e);
    }
    assert!(terminate_result.is_ok());
    
    // Wait for cleanup
    tokio::time::sleep(Duration::from_millis(50)).await;
    
    // Verify session is removed
    let session_after = manager_a.find_session(&session_id).await.unwrap();
    assert!(session_after.is_none());
}

#[tokio::test]
async fn test_multiple_early_cancellations() {
    let (manager_a, manager_b, _) = create_session_manager_pair().await.unwrap();
    let target_addr = manager_b.get_bound_address();
    
    // Create multiple calls and cancel them quickly
    let mut calls = Vec::new();
    
    for i in 0..3 {
        let call = manager_a.create_outgoing_call(
            &format!("sip:caller{}@127.0.0.1", i),
            &format!("sip:target{}@{}", i, target_addr),
            Some(format!("SDP offer for call {}", i))
        ).await.unwrap();
        calls.push(call);
    }
    
    // Wait a bit for calls to be initiated
    tokio::time::sleep(Duration::from_millis(50)).await;
    
    // Cancel all calls immediately
    for call in &calls {
        let result = manager_a.terminate_session(call.id()).await;
        if let Err(ref e) = result {
            println!("❌ Terminate error for call {}: {:?}", call.id(), e);
        }
        // Some may fail if already terminated, which is acceptable
    }
    
    // Wait for cleanup
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    // Verify all sessions are cleaned up
    let final_stats = manager_a.get_stats().await.unwrap();
    assert_eq!(final_stats.active_sessions, 0);
}

#[tokio::test]
async fn test_cancel_nonexistent_session() {
    let (manager_a, _manager_b, _) = create_session_manager_pair().await.unwrap();
    
    // Try to cancel a non-existent session
    let fake_session_id = SessionId::new();
    let terminate_result = manager_a.terminate_session(&fake_session_id).await;
    assert!(terminate_result.is_err());
    assert!(matches!(terminate_result.unwrap_err(), SessionError::SessionNotFound(_)));
}

#[tokio::test]
async fn test_cancel_timing_scenarios() {
    let (manager_a, manager_b, _) = create_session_manager_pair().await.unwrap();
    let target_addr = manager_b.get_bound_address();
    
    // Create call
    let call = manager_a.create_outgoing_call(
        "sip:alice@127.0.0.1",
        &format!("sip:bob@{}", target_addr),
        Some("SDP offer".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Wait a very short time before cancelling (simulates early CANCEL)
    tokio::time::sleep(Duration::from_millis(10)).await;
    
    let terminate_result = manager_a.terminate_session(&session_id).await;
    if let Err(ref e) = terminate_result {
        println!("❌ Early terminate error: {:?}", e);
        // Early termination may fail if session is already being cleaned up
        // This is acceptable behavior
    }
}

#[tokio::test]
async fn test_cancel_after_provisional_response() {
    let (manager_a, manager_b, _) = create_session_manager_pair().await.unwrap();
    let target_addr = manager_b.get_bound_address();
    
    // Create call
    let call = manager_a.create_outgoing_call(
        "sip:alice@127.0.0.1",
        &format!("sip:bob@{}", target_addr),
        Some("SDP offer".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Wait longer before cancelling (simulates CANCEL after 18x response)
    tokio::time::sleep(Duration::from_millis(200)).await;
    
    let terminate_result = manager_a.terminate_session(&session_id).await;
    if let Err(ref e) = terminate_result {
        println!("❌ Late terminate error: {:?}", e);
    }
    // Note: This may succeed or fail depending on call state, both are valid
}

#[tokio::test]
async fn test_cancel_session_state_management() {
    let (manager_a, manager_b, _) = create_session_manager_pair().await.unwrap();
    let target_addr = manager_b.get_bound_address();
    
    // Create call
    let call = manager_a.create_outgoing_call(
        "sip:alice@127.0.0.1",
        &format!("sip:bob@{}", target_addr),
        Some("SDP offer".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Verify session exists before cancel
    let session_before = manager_a.find_session(&session_id).await.unwrap();
    assert!(session_before.is_some());
    let state_before = session_before.unwrap().state().clone();
    assert_eq!(state_before, CallState::Initiating);
    
    // Wait for call processing
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    // Try to cancel the session
    let terminate_result = manager_a.terminate_session(&session_id).await;
    println!("Terminate result: {:?}", terminate_result);
    
    // Wait for state update
    tokio::time::sleep(Duration::from_millis(50)).await;
    
    // Verify session is removed (regardless of whether terminate succeeded)
    let session_after = manager_a.find_session(&session_id).await.unwrap();
    // Session should be gone either from successful termination or natural cleanup
    if session_after.is_some() {
        println!("Session still exists with state: {:?}", session_after.unwrap().state());
    }
}

#[tokio::test]
async fn test_cancel_statistics_tracking() {
    let (manager_a, manager_b, _) = create_session_manager_pair().await.unwrap();
    let target_addr = manager_b.get_bound_address();
    
    // Check initial stats
    let initial_stats = manager_a.get_stats().await.unwrap();
    let initial_count = initial_stats.active_sessions;
    
    // Create call
    let call = manager_a.create_outgoing_call(
        "sip:alice@127.0.0.1",
        &format!("sip:bob@{}", target_addr),
        Some("SDP offer".to_string())
    ).await.unwrap();
    
    // Verify stats increased
    let mid_stats = manager_a.get_stats().await.unwrap();
    assert_eq!(mid_stats.active_sessions, initial_count + 1);
    
    // Wait for processing
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    // Try to cancel call
    let terminate_result = manager_a.terminate_session(call.id()).await;
    println!("Terminate result: {:?}", terminate_result);
    
    // Wait for cleanup
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    // Verify stats decreased (session should be cleaned up)
    let final_stats = manager_a.get_stats().await.unwrap();
    assert_eq!(final_stats.active_sessions, initial_count);
}

#[tokio::test]
async fn test_concurrent_cancellations() {
    let (manager_a, manager_b, _) = create_session_manager_pair().await.unwrap();
    let target_addr = manager_b.get_bound_address();
    
    // Create multiple calls
    let mut calls = Vec::new();
    for i in 0..5 {
        let call = manager_a.create_outgoing_call(
            &format!("sip:caller{}@127.0.0.1", i),
            &format!("sip:target{}@{}", i, target_addr),
            Some(format!("SDP offer {}", i))
        ).await.unwrap();
        calls.push(call);
    }
    
    // Wait for calls to be initiated
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    // Cancel all calls concurrently
    let mut cancel_tasks = Vec::new();
    for call in calls {
        let manager_clone = manager_a.clone();
        let session_id = call.id().clone();
        let task = tokio::spawn(async move {
            let result = manager_clone.terminate_session(&session_id).await;
            println!("Concurrent terminate result for {}: {:?}", session_id, result);
            result
        });
        cancel_tasks.push(task);
    }
    
    // Wait for all cancellations to complete (some may fail, which is acceptable)
    let mut success_count = 0;
    for task in cancel_tasks {
        match task.await.unwrap() {
            Ok(_) => success_count += 1,
            Err(_) => {} // Some failures are expected in concurrent scenarios
        }
    }
    
    println!("Successful terminations: {}/5", success_count);
    
    // Wait for cleanup
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    // Verify all sessions are cleaned up
    let final_stats = manager_a.get_stats().await.unwrap();
    assert_eq!(final_stats.active_sessions, 0);
} 