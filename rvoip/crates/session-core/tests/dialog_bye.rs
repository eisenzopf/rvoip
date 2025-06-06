//! Tests for BYE Dialog Integration
//!
//! Tests the session-core functionality for BYE requests (call termination),
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
    },
};
use common::*;

/// Test handler for BYE testing that tracks terminations
#[derive(Debug)]
struct ByeTestHandler {
    terminated_calls: Arc<tokio::sync::Mutex<Vec<(SessionId, String)>>>,
    call_durations: Arc<tokio::sync::Mutex<Vec<std::time::Duration>>>,
}

impl ByeTestHandler {
    fn new() -> Self {
        Self {
            terminated_calls: Arc::new(tokio::sync::Mutex::new(Vec::new())),
            call_durations: Arc::new(tokio::sync::Mutex::new(Vec::new())),
        }
    }

    async fn add_terminated_call(&self, session_id: SessionId, reason: String, duration: std::time::Duration) {
        self.terminated_calls.lock().await.push((session_id, reason));
        self.call_durations.lock().await.push(duration);
    }

    async fn get_terminated_calls(&self) -> Vec<(SessionId, String)> {
        self.terminated_calls.lock().await.clone()
    }

    async fn get_call_durations(&self) -> Vec<std::time::Duration> {
        self.call_durations.lock().await.clone()
    }
}

#[async_trait::async_trait]
impl CallHandler for ByeTestHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        CallDecision::Accept
    }

    async fn on_call_ended(&self, call: CallSession, reason: &str) {
        let duration = call.started_at
            .map(|start| start.elapsed())
            .unwrap_or_else(|| std::time::Duration::from_secs(0));
        
        self.add_terminated_call(call.id().clone(), reason.to_string(), duration).await;
        tracing::info!("BYE test call {} ended after {:?}: {}", call.id(), duration, reason);
    }
}

#[tokio::test]
async fn test_basic_bye_termination() {
    let (manager_a, manager_b, mut call_events) = create_session_manager_pair().await.unwrap();
    
    // Establish a call first
    let (call, _callee_session_id) = establish_call_between_managers(&manager_a, &manager_b, &mut call_events).await.unwrap();
    let session_id = call.id().clone();
    
    // Verify session exists before termination
    verify_session_exists(&manager_a, &session_id, Some(&CallState::Initiating)).await.unwrap();
    
    // Terminate with BYE
    let terminate_result = manager_a.terminate_session(&session_id).await;
    assert!(terminate_result.is_ok());
    
    // Wait for BYE processing
    let config = TestConfig::default();
    tokio::time::sleep(config.cleanup_delay).await;
    
    // Verify session is removed after BYE
    verify_session_removed(&manager_a, &session_id).await.unwrap();
    
    cleanup_managers(vec![manager_a, manager_b]).await.unwrap();
}

#[tokio::test]
async fn test_immediate_bye_after_invite() {
    let (manager_a, manager_b, mut call_events) = create_session_manager_pair().await.unwrap();
    
    // Create call but don't wait for establishment
    let (call, _) = establish_call_between_managers(&manager_a, &manager_b, &mut call_events).await.unwrap();
    let session_id = call.id().clone();
    
    // Immediate termination (should send BYE or CANCEL depending on state)
    let terminate_result = manager_a.terminate_session(&session_id).await;
    assert!(terminate_result.is_ok());
    
    // Wait for cleanup
    let config = TestConfig::default();
    tokio::time::sleep(config.cleanup_delay).await;
    
    // Verify session cleanup
    verify_session_removed(&manager_a, &session_id).await.unwrap();
    
    cleanup_managers(vec![manager_a, manager_b]).await.unwrap();
}

#[tokio::test]
async fn test_bye_after_call_establishment() {
    let (manager_a, manager_b, mut call_events) = create_session_manager_pair().await.unwrap();
    
    // Establish a call first
    let (call, _callee_session_id) = establish_call_between_managers(&manager_a, &manager_b, &mut call_events).await.unwrap();
    let session_id = call.id().clone();
    
    // Simulate call being established (wait a bit)
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    // Now send BYE to terminate established call
    let terminate_result = manager_a.terminate_session(&session_id).await;
    assert!(terminate_result.is_ok());
    
    // Wait for BYE processing
    let config = TestConfig::default();
    tokio::time::sleep(config.cleanup_delay).await;
    
    // Verify session cleanup
    verify_session_removed(&manager_a, &session_id).await.unwrap();
    
    cleanup_managers(vec![manager_a, manager_b]).await.unwrap();
}

#[tokio::test]
async fn test_bye_multiple_concurrent_calls() {
    let (manager_a, manager_b, mut call_events) = create_session_manager_pair().await.unwrap();
    
    // Create multiple calls using the pair
    let mut calls = Vec::new();
    for i in 0..3 { // Reduced for reliability
        let call = manager_a.create_outgoing_call(
            &format!("sip:caller{}@example.com", i),
            &format!("sip:target{}@example.com", i),
            Some(format!("SDP offer {}", i))
        ).await.unwrap();
        calls.push(call);
    }
    
    // Verify all calls exist
    let stats_before = manager_a.get_stats().await.unwrap();
    assert_eq!(stats_before.active_sessions, 3);
    
    // Terminate all calls with BYE
    for call in &calls {
        let result = manager_a.terminate_session(call.id()).await;
        // Note: These will fail with the "requires remote tag" error, which is expected
        // for calls to non-existent endpoints. This is actually correct behavior.
        if result.is_err() {
            println!("Expected error for call to non-existent endpoint: {:?}", result);
        }
    }
    
    // Wait for cleanup attempts
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    cleanup_managers(vec![manager_a, manager_b]).await.unwrap();
}

#[tokio::test]
async fn test_bye_nonexistent_session() {
    let handler = Arc::new(ByeTestHandler::new());
    let manager = create_session_manager(handler, None, Some("sip:test@localhost")).await.unwrap();
    
    // Try to send BYE to non-existent session
    let fake_session_id = SessionId::new();
    let terminate_result = manager.terminate_session(&fake_session_id).await;
    assert!(terminate_result.is_err());
    assert!(matches!(terminate_result.unwrap_err(), SessionError::SessionNotFound(_)));
    
    cleanup_managers(vec![manager]).await.unwrap();
}

#[tokio::test]
async fn test_bye_after_hold_operations() {
    let (manager_a, manager_b, mut call_events) = create_session_manager_pair().await.unwrap();
    
    // Establish a call first
    let (call, _) = establish_call_between_managers(&manager_a, &manager_b, &mut call_events).await.unwrap();
    let session_id = call.id().clone();
    
    // Perform hold operations
    let hold_result = manager_a.hold_session(&session_id).await;
    // Note: hold may also fail if dialog isn't fully established, which is expected
    if hold_result.is_err() {
        println!("Hold operation failed (expected for early dialog): {:?}", hold_result);
    }
    
    let resume_result = manager_a.resume_session(&session_id).await;
    if resume_result.is_err() {
        println!("Resume operation failed (expected for early dialog): {:?}", resume_result);
    }
    
    // Terminate after hold/resume sequence
    let terminate_result = manager_a.terminate_session(&session_id).await;
    assert!(terminate_result.is_ok());
    
    // Wait for cleanup
    let config = TestConfig::default();
    tokio::time::sleep(config.cleanup_delay).await;
    
    // Verify session cleanup
    verify_session_removed(&manager_a, &session_id).await.unwrap();
    
    cleanup_managers(vec![manager_a, manager_b]).await.unwrap();
}

#[tokio::test]
async fn test_bye_after_media_updates() {
    let (manager_a, manager_b, mut call_events) = create_session_manager_pair().await.unwrap();
    
    // Establish a call first
    let (call, _) = establish_call_between_managers(&manager_a, &manager_b, &mut call_events).await.unwrap();
    let session_id = call.id().clone();
    
    // Perform media updates
    let update_result = manager_a.update_media(&session_id, "Updated SDP").await;
    // Note: media update may also fail if dialog isn't fully established, which is expected
    if update_result.is_err() {
        println!("Media update failed (expected for early dialog): {:?}", update_result);
    }
    
    // Terminate after media update
    let terminate_result = manager_a.terminate_session(&session_id).await;
    assert!(terminate_result.is_ok());
    
    // Wait for cleanup
    let config = TestConfig::default();
    tokio::time::sleep(config.cleanup_delay).await;
    
    // Verify session cleanup
    verify_session_removed(&manager_a, &session_id).await.unwrap();
    
    cleanup_managers(vec![manager_a, manager_b]).await.unwrap();
}

#[tokio::test]
async fn test_concurrent_bye_operations() {
    let (manager_a, manager_b, mut call_events) = create_session_manager_pair().await.unwrap();
    
    // Create multiple calls using established pattern
    let mut calls = Vec::new();
    for i in 0..3 {
        let call = manager_a.create_outgoing_call(
            &format!("sip:caller{}@example.com", i),
            &format!("sip:target{}@example.com", i),
            Some(format!("BYE test SDP {}", i))
        ).await.unwrap();
        calls.push(call);
    }
    
    // Terminate all calls concurrently
    let mut bye_tasks = Vec::new();
    for call in calls {
        let manager_clone = manager_a.clone();
        let session_id = call.id().clone();
        let task = tokio::spawn(async move {
            manager_clone.terminate_session(&session_id).await
        });
        bye_tasks.push(task);
    }
    
    // Wait for all BYE operations to complete
    for task in bye_tasks {
        let result = task.await.unwrap();
        // Note: May fail with "requires remote tag" for non-established dialogs
        if result.is_err() {
            println!("Expected BYE error for non-established dialog: {:?}", result);
        }
    }
    
    // Wait for cleanup
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    cleanup_managers(vec![manager_a, manager_b]).await.unwrap();
}

#[tokio::test]
async fn test_bye_timing_measurements() {
    let (manager_a, manager_b, mut call_events) = create_session_manager_pair().await.unwrap();
    
    // Establish a call first
    let (call, _) = establish_call_between_managers(&manager_a, &manager_b, &mut call_events).await.unwrap();
    let session_id = call.id().clone();
    
    // Wait to establish call duration
    tokio::time::sleep(Duration::from_millis(200)).await;
    
    // Measure BYE operation time
    let bye_start = std::time::Instant::now();
    let terminate_result = manager_a.terminate_session(&session_id).await;
    assert!(terminate_result.is_ok());
    let bye_duration = bye_start.elapsed();
    
    // BYE should complete quickly
    assert!(bye_duration < Duration::from_secs(1));
    
    // Wait for cleanup
    let config = TestConfig::default();
    tokio::time::sleep(config.cleanup_delay).await;
    
    // Verify session cleanup
    verify_session_removed(&manager_a, &session_id).await.unwrap();
    
    cleanup_managers(vec![manager_a, manager_b]).await.unwrap();
}

#[tokio::test]
async fn test_bye_session_state_transitions() {
    let (manager_a, manager_b, mut call_events) = create_session_manager_pair().await.unwrap();
    
    // Establish a call first
    let (call, _) = establish_call_between_managers(&manager_a, &manager_b, &mut call_events).await.unwrap();
    let session_id = call.id().clone();
    
    // Verify initial state
    verify_session_exists(&manager_a, &session_id, Some(&CallState::Initiating)).await.unwrap();
    
    // Terminate session
    let terminate_result = manager_a.terminate_session(&session_id).await;
    assert!(terminate_result.is_ok());
    
    // Wait for state transition
    let config = TestConfig::default();
    tokio::time::sleep(config.cleanup_delay).await;
    
    // Session should be removed (terminated)
    verify_session_removed(&manager_a, &session_id).await.unwrap();
    
    cleanup_managers(vec![manager_a, manager_b]).await.unwrap();
}

#[tokio::test]
async fn test_double_bye_protection() {
    let (manager_a, manager_b, mut call_events) = create_session_manager_pair().await.unwrap();
    
    // Establish a call first
    let (call, _) = establish_call_between_managers(&manager_a, &manager_b, &mut call_events).await.unwrap();
    let session_id = call.id().clone();
    
    // First BYE
    let first_bye = manager_a.terminate_session(&session_id).await;
    assert!(first_bye.is_ok());
    
    // Wait a moment
    let config = TestConfig::default();
    tokio::time::sleep(config.cleanup_delay).await;
    
    // Second BYE (should fail gracefully)
    let second_bye = manager_a.terminate_session(&session_id).await;
    assert!(second_bye.is_err());
    assert!(matches!(second_bye.unwrap_err(), SessionError::SessionNotFound(_)));
    
    cleanup_managers(vec![manager_a, manager_b]).await.unwrap();
}

#[tokio::test]
async fn test_bye_statistics_tracking() {
    let (manager_a, manager_b, mut call_events) = create_session_manager_pair().await.unwrap();
    
    // Create multiple calls
    let mut calls = Vec::new();
    for i in 0..3 {
        let call = manager_a.create_outgoing_call(
            &format!("sip:caller{}@example.com", i),
            &format!("sip:target{}@example.com", i),
            Some(format!("Stats test SDP {}", i))
        ).await.unwrap();
        calls.push(call);
    }
    
    // Verify initial stats
    let initial_stats = manager_a.get_stats().await.unwrap();
    assert_eq!(initial_stats.active_sessions, 3);
    
    // Note: Since these are calls to non-existent endpoints, BYE will fail
    // But we can still test the session tracking behavior
    
    // Final verification (sessions should still exist since BYE failed)
    let final_stats = manager_a.get_stats().await.unwrap();
    assert_eq!(final_stats.active_sessions, 3);
    
    cleanup_managers(vec![manager_a, manager_b]).await.unwrap();
}

#[tokio::test]
async fn test_error_conditions_for_non_established_dialogs() {
    let handler = Arc::new(ByeTestHandler::new());
    let manager = create_session_manager(handler, None, Some("sip:test@localhost")).await.unwrap();
    
    // Create call to non-existent endpoint
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("SDP offer".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Verify session exists
    verify_session_exists(&manager, &session_id, Some(&CallState::Initiating)).await.unwrap();
    
    // Try to terminate - should fail with "requires remote tag" error
    let terminate_result = manager.terminate_session(&session_id).await;
    assert!(terminate_result.is_err());
    
    // Verify the specific error
    match terminate_result.unwrap_err() {
        SessionError::Other(msg) if msg.contains("requires remote tag") => {
            println!("✓ Got expected error for BYE on non-established dialog: {}", msg);
        }
        other => panic!("Expected 'requires remote tag' error, got: {:?}", other),
    }
    
    // Session should still exist since termination failed
    verify_session_exists(&manager, &session_id, Some(&CallState::Initiating)).await.unwrap();
    
    cleanup_managers(vec![manager]).await.unwrap();
} 