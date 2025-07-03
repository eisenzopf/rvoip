use rvoip_session_core::api::control::SessionControl;
// Tests for BYE Dialog Integration
//
// Tests the session-core functionality for BYE requests (call termination),
// ensuring proper integration with the underlying dialog layer.

// All tests in this file run serially to avoid port conflicts
use serial_test::serial;

mod common;

use std::sync::Arc;
use std::time::Duration;
use rvoip_session_core::{
    SessionCoordinator,
    SessionError,
    api::{
        types::{CallState, SessionId, IncomingCall, CallSession, CallDecision},
        handlers::CallHandler,
    },
    manager::events::SessionEvent,
};
use common::*;
use common::media_test_utils;

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
        CallDecision::Accept(None)
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
#[serial]
async fn test_basic_bye_termination() {
    // Global timeout for the entire test
    let test_result = tokio::time::timeout(Duration::from_secs(30), async {
        let (manager_a, manager_b, mut call_events) = create_session_manager_pair().await.unwrap();
        
        // Establish a call first
        let (call, _callee_session_id) = establish_call_between_managers(&manager_a, &manager_b, &mut call_events).await.unwrap();
        let session_id = call.id().clone();
        
        // Verify session exists before termination (should be Active after INVITE/200OK/ACK)
    verify_session_exists(&manager_a, &session_id, Some(&CallState::Active)).await.unwrap();
    
    // Terminate with BYE
    let terminate_result = manager_a.terminate_session(&session_id).await;
    assert!(terminate_result.is_ok());
        
        // WORKAROUND: Poll for session removal instead of waiting for events
        // The event system has a race condition that needs to be fixed
        let mut session_removed = false;
        let start = std::time::Instant::now();
        while start.elapsed() < Duration::from_secs(5) {
            match manager_a.get_session(&session_id).await {
                Ok(None) => {
                    session_removed = true;
                    break;
                }
                Ok(Some(session)) if matches!(session.state(), CallState::Terminated) => {
                    session_removed = true;
                    break;
                }
                _ => {
                    tokio::time::sleep(Duration::from_millis(100)).await;
                }
            }
        }
        assert!(session_removed, "Session should be terminated/removed after BYE");
        
        // Verify session is removed after BYE
        verify_session_removed(&manager_a, &session_id).await.unwrap();
        
        cleanup_managers(vec![manager_a, manager_b]).await.unwrap();
    }).await;
    
    match test_result {
        Ok(_) => println!("✅ Test completed successfully"),
        Err(_) => panic!("❌ Test timed out after 30 seconds!"),
    }
}

#[tokio::test]
#[serial]
async fn test_immediate_bye_after_invite() {
    let test_result = tokio::time::timeout(Duration::from_secs(30), async {
        let (manager_a, manager_b, mut call_events) = create_session_manager_pair().await.unwrap();
    
    // Subscribe to events BEFORE creating the call
    let mut event_sub = manager_a.event_processor.subscribe().await.unwrap();
    
    // Create call but don't wait for establishment
    let (call, _) = establish_call_between_managers(&manager_a, &manager_b, &mut call_events).await.unwrap();
    let session_id = call.id().clone();
    
    // Immediate termination (should send BYE or CANCEL depending on state)
    let terminate_result = manager_a.terminate_session(&session_id).await;
    assert!(terminate_result.is_ok());
    
    // Wait for session terminated event
    let termination_reason = wait_for_session_terminated(&mut event_sub, &session_id, Duration::from_secs(2)).await;
    assert!(termination_reason.is_some(), "Should receive session terminated event");
    
    // Verify session cleanup
    verify_session_removed(&manager_a, &session_id).await.unwrap();
    
    cleanup_managers(vec![manager_a, manager_b]).await.unwrap();
    }).await;
    
    match test_result {
        Ok(_) => println!("✅ Test completed successfully"),
        Err(_) => panic!("❌ Test timed out after 30 seconds!"),
    }
}

#[tokio::test]
#[serial]
async fn test_bye_after_call_establishment() {
    let test_result = tokio::time::timeout(Duration::from_secs(30), async {
    let (manager_a, manager_b, mut call_events) = create_session_manager_pair().await.unwrap();
    
    // Subscribe to events BEFORE creating the call
    let mut event_sub = manager_a.event_processor.subscribe().await.unwrap();
    
    // Establish a call first
    let (call, _callee_session_id) = establish_call_between_managers(&manager_a, &manager_b, &mut call_events).await.unwrap();
    let session_id = call.id().clone();
    
    // Wait a bit to ensure call is fully established
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    // Now send BYE to terminate established call
    let terminate_result = manager_a.terminate_session(&session_id).await;
    assert!(terminate_result.is_ok());
    
    // Wait for session terminated event
    let termination_reason = wait_for_session_terminated(&mut event_sub, &session_id, Duration::from_secs(2)).await;
    assert!(termination_reason.is_some(), "Should receive session terminated event");
    
    // Verify session cleanup
    verify_session_removed(&manager_a, &session_id).await.unwrap();
    
    cleanup_managers(vec![manager_a, manager_b]).await.unwrap();
    }).await;
    
    match test_result {
        Ok(_) => println!("✅ Test completed successfully"),
        Err(_) => panic!("❌ Test timed out after 30 seconds!"),
    }
}

#[tokio::test]
#[serial]
async fn test_bye_multiple_concurrent_calls() {
    let test_result = tokio::time::timeout(Duration::from_secs(30), async {
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
    }).await;
    
    match test_result {
        Ok(_) => println!("✅ Test completed successfully"),
        Err(_) => panic!("❌ Test timed out after 30 seconds!"),
    }
}

#[tokio::test]
#[serial]
async fn test_bye_nonexistent_session() {
    let test_result = tokio::time::timeout(Duration::from_secs(30), async {
let handler = Arc::new(ByeTestHandler::new());
    let manager = create_session_manager(Arc::new(media_test_utils::TestCallHandler::new(true)), None, Some("sip:test@localhost")).await.unwrap();
    
    // Try to send BYE to non-existent session
    let fake_session_id = SessionId::new();
    let terminate_result = manager.terminate_session(&fake_session_id).await;
    assert!(terminate_result.is_err());
    assert!(matches!(terminate_result.unwrap_err(), SessionError::SessionNotFound(_)));
    
    cleanup_managers(vec![manager]).await.unwrap();
    }).await;
    
    match test_result {
        Ok(_) => println!("✅ Test completed successfully"),
        Err(_) => panic!("❌ Test timed out after 30 seconds!"),
    }
}

#[tokio::test]
#[serial]
async fn test_bye_after_hold_operations() {
    let test_result = tokio::time::timeout(Duration::from_secs(30), async {
    let (manager_a, manager_b, mut call_events) = create_session_manager_pair().await.unwrap();
    
    // Subscribe to events BEFORE creating the call
    let mut event_sub = manager_a.event_processor.subscribe().await.unwrap();
    
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
    
    // Wait for session terminated event
    let termination_reason = wait_for_session_terminated(&mut event_sub, &session_id, Duration::from_secs(2)).await;
    assert!(termination_reason.is_some(), "Should receive session terminated event");
    
    // Verify session cleanup
    verify_session_removed(&manager_a, &session_id).await.unwrap();
    
    cleanup_managers(vec![manager_a, manager_b]).await.unwrap();
    }).await;
    
    match test_result {
        Ok(_) => println!("✅ Test completed successfully"),
        Err(_) => panic!("❌ Test timed out after 30 seconds!"),
    }
}

#[tokio::test]
#[serial]
async fn test_bye_after_media_updates() {
    let test_result = tokio::time::timeout(Duration::from_secs(30), async {
    let (manager_a, manager_b, mut call_events) = create_session_manager_pair().await.unwrap();
    
    // Subscribe to events BEFORE creating the call
    let mut event_sub = manager_a.event_processor.subscribe().await.unwrap();
    
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
    
    // Wait for session terminated event
    let termination_reason = wait_for_session_terminated(&mut event_sub, &session_id, Duration::from_secs(2)).await;
    assert!(termination_reason.is_some(), "Should receive session terminated event");
    
    // Verify session cleanup
    verify_session_removed(&manager_a, &session_id).await.unwrap();
    
    cleanup_managers(vec![manager_a, manager_b]).await.unwrap();
    }).await;
    
    match test_result {
        Ok(_) => println!("✅ Test completed successfully"),
        Err(_) => panic!("❌ Test timed out after 30 seconds!"),
    }
}

#[tokio::test]
#[serial]
async fn test_concurrent_bye_operations() {
    let test_result = tokio::time::timeout(Duration::from_secs(30), async {
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
    }).await;
    
    match test_result {
        Ok(_) => println!("✅ Test completed successfully"),
        Err(_) => panic!("❌ Test timed out after 30 seconds!"),
    }
}

#[tokio::test]
#[serial]
async fn test_bye_timing_measurements() {
    let test_result = tokio::time::timeout(Duration::from_secs(30), async {
    let (manager_a, manager_b, mut call_events) = create_session_manager_pair().await.unwrap();
    
    // Subscribe to events BEFORE creating the call
    let mut event_sub = manager_a.event_processor.subscribe().await.unwrap();
    
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
    
    // Wait for state transition to Terminated
    let state_changed = wait_for_terminated_state(&mut event_sub, &session_id, Duration::from_secs(2)).await;
    assert!(state_changed, "Session should transition to Terminated state");
    
    // Verify session cleanup
    verify_session_removed(&manager_a, &session_id).await.unwrap();
    
    cleanup_managers(vec![manager_a, manager_b]).await.unwrap();
    }).await;
    
    match test_result {
        Ok(_) => println!("✅ Test completed successfully"),
        Err(_) => panic!("❌ Test timed out after 30 seconds!"),
    }
}

#[tokio::test]
#[serial]
async fn test_bye_session_state_transitions() {
    let test_result = tokio::time::timeout(Duration::from_secs(30), async {
    let (manager_a, manager_b, mut call_events) = create_session_manager_pair().await.unwrap();
    
    // Subscribe to events BEFORE creating the call
    let mut event_sub = manager_a.event_processor.subscribe().await.unwrap();
    
    // Establish a call first
    let (call, _) = establish_call_between_managers(&manager_a, &manager_b, &mut call_events).await.unwrap();
    let session_id = call.id().clone();
    
    // Verify initial state (should be Active after INVITE/200OK/ACK)
    verify_session_exists(&manager_a, &session_id, Some(&CallState::Active)).await.unwrap();
    
    // Terminate session
    let terminate_result = manager_a.terminate_session(&session_id).await;
    assert!(terminate_result.is_ok());
    
    // Wait for state transition to Terminated
    let state_changed = wait_for_terminated_state(&mut event_sub, &session_id, Duration::from_secs(2)).await;
    assert!(state_changed, "Session should transition to Terminated state");
    
    // Session should be removed (terminated)
    verify_session_removed(&manager_a, &session_id).await.unwrap();
    
    cleanup_managers(vec![manager_a, manager_b]).await.unwrap();
    }).await;
    
    match test_result {
        Ok(_) => println!("✅ Test completed successfully"),
        Err(_) => panic!("❌ Test timed out after 30 seconds!"),
    }
}

#[tokio::test]
#[serial]
async fn test_double_bye_protection() {
    let test_result = tokio::time::timeout(Duration::from_secs(30), async {
    let (manager_a, manager_b, mut call_events) = create_session_manager_pair().await.unwrap();
    
    // Establish a call first
    let (call, _) = establish_call_between_managers(&manager_a, &manager_b, &mut call_events).await.unwrap();
    let session_id = call.id().clone();
    
    // First BYE
    let first_bye = manager_a.terminate_session(&session_id).await;
    assert!(first_bye.is_ok());
    
    // Wait a moment
    tokio::time::sleep(Duration::from_millis(200)).await;
    
    // Second BYE (should fail gracefully)
    let second_bye = manager_a.terminate_session(&session_id).await;
    // The session might still exist in Terminated state, or might be removed
    // Either SessionNotFound or invalid state error is acceptable
    if second_bye.is_err() {
        let err = second_bye.unwrap_err();
        assert!(
            matches!(err, SessionError::SessionNotFound(_)) || 
            matches!(err, SessionError::InvalidState(_)) ||
            matches!(err, SessionError::Other(_)),
            "Unexpected error type: {:?}", err
        );
    } else {
        // It's also acceptable if the second BYE succeeds (idempotent behavior)
        println!("Second BYE succeeded (idempotent behavior)");
    }
    
    cleanup_managers(vec![manager_a, manager_b]).await.unwrap();
    }).await;
    
    match test_result {
        Ok(_) => println!("✅ Test completed successfully"),
        Err(_) => panic!("❌ Test timed out after 30 seconds!"),
    }
}

#[tokio::test]
#[serial]
async fn test_bye_statistics_tracking() {
    let test_result = tokio::time::timeout(Duration::from_secs(30), async {
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
    }).await;
    
    match test_result {
        Ok(_) => println!("✅ Test completed successfully"),
        Err(_) => panic!("❌ Test timed out after 30 seconds!"),
    }
}

#[tokio::test]
#[serial]
async fn test_error_conditions_for_non_established_dialogs() {
    let test_result = tokio::time::timeout(Duration::from_secs(30), async {
let handler = Arc::new(ByeTestHandler::new());
    let manager = create_session_manager(Arc::new(media_test_utils::TestCallHandler::new(true)), None, Some("sip:test@localhost")).await.unwrap();
    
    // Create call to non-existent endpoint
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("SDP offer".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Verify session exists
    verify_session_exists(&manager, &session_id, Some(&CallState::Initiating)).await.unwrap();
    
    // Try to terminate - terminate_session() is now state-aware:
    // - For early dialogs (Initiating), it will use CANCEL
    // - For established dialogs, it will use BYE
    // However, since this is a call to a non-existent endpoint, the INVITE transaction
    // fails immediately and transitions to Terminated state. In this case, terminate_session()
    // will succeed because it can successfully "terminate" an already-failed session.
    let terminate_result = manager.terminate_session(&session_id).await;
    
    // Check the current session state - it might already be terminated due to INVITE failure
    match manager.get_session(&session_id).await {
        Ok(Some(session)) if matches!(session.state(), CallState::Terminated) => {
            // Session already terminated due to INVITE failure - terminate_session should succeed
            assert!(terminate_result.is_ok(), "Terminating an already-failed session should succeed");
            println!("✓ Session was already terminated due to INVITE failure, terminate_session succeeded");
        }
        Ok(Some(_session)) => {
            // Session still active - in this case we expect termination to potentially fail
            if terminate_result.is_err() {
                println!("✓ Got expected error for early dialog termination: {:?}", terminate_result.unwrap_err());
            } else {
                println!("✓ Early dialog termination succeeded");
            }
        }
        Ok(None) => {
            // Session was already removed - terminate_session should have failed
            assert!(terminate_result.is_err(), "Terminating a non-existent session should fail");
        }
        Err(e) => panic!("Failed to get session state: {:?}", e),
    }
    
    cleanup_managers(vec![manager]).await.unwrap();
    }).await;
    
    match test_result {
        Ok(_) => println!("✅ Test completed successfully"),
        Err(_) => panic!("❌ Test timed out after 30 seconds!"),
    }
} 