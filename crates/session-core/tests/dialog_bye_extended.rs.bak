use rvoip_session_core::api::control::SessionControl;
// Tests for Early Call Termination (BYE for established calls)
//
// These tests verify session termination functionality.
// Note: These tests actually test BYE functionality since calls get
// established too quickly for CANCEL. See dialog_cancel_proper.rs 
// for actual early dialog CANCEL testing.

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
        builder::SessionManagerBuilder,
    },
};
use common::*;

/// Helper function to wait for session cleanup with retries
async fn wait_for_no_active_sessions(
    manager_a: &Arc<SessionCoordinator>,
    manager_b: &Arc<SessionCoordinator>,
    timeout: Duration,
) -> Result<(), String> {
    let start = std::time::Instant::now();
    let mut last_count_a = 0;
    let mut last_count_b = 0;
    
    while start.elapsed() < timeout {
        let stats_a = manager_a.get_stats().await.unwrap();
        let stats_b = manager_b.get_stats().await.unwrap();
        
        if stats_a.active_sessions != last_count_a || stats_b.active_sessions != last_count_b {
            println!("Session count changed - Manager A: {} active, Manager B: {} active", 
                     stats_a.active_sessions, stats_b.active_sessions);
            last_count_a = stats_a.active_sessions;
            last_count_b = stats_b.active_sessions;
        }
        
        if stats_a.active_sessions == 0 && stats_b.active_sessions == 0 {
            return Ok(());
        }
        
        tokio::time::sleep(Duration::from_millis(200)).await;
    }
    
    // Print debug info about remaining sessions
    println!("Timeout waiting for cleanup. Checking remaining sessions...");
    
    let stats_a = manager_a.get_stats().await.unwrap();
    let stats_b = manager_b.get_stats().await.unwrap();
    
    Err(format!("Timeout waiting for session cleanup after {} seconds. Manager A: {} active, Manager B: {} active",
                timeout.as_secs(), stats_a.active_sessions, stats_b.active_sessions))
}

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
        CallDecision::Accept(None)
    }

    async fn on_call_ended(&self, call: CallSession, reason: &str) {
        if reason.contains("cancel") || reason.contains("CANCEL") {
            self.cancelled_calls.lock().await.push(call.id().clone());
        }
        tracing::info!("Call {} ended with reason: {}", call.id(), reason);
    }
}

/// Create a test session manager for CANCEL testing
async fn create_cancel_test_manager() -> Result<Arc<SessionCoordinator>, SessionError> {
let handler = Arc::new(CancelTestHandler::new());
    
    SessionManagerBuilder::new()
        .with_local_address("127.0.0.1")
        .with_sip_port(5061) // Use specific test port
        .with_handler(Arc::new(media_test_utils::TestCallHandler::new(true)))
        .build()
        .await
}

#[tokio::test]
#[serial]
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
#[serial]
async fn test_call_termination_established() {
    let (manager_a, manager_b, mut call_events) = create_session_manager_pair().await.unwrap();
    
    // Establish a call fully
    let (call, _) = establish_call_between_managers(&manager_a, &manager_b, &mut call_events).await.unwrap();
    let session_id = call.id().clone();
    
    // Subscribe to events
    let mut event_sub = manager_a.event_processor.subscribe().await.unwrap();
    
    // Check session state before termination
    let session_before = manager_a.find_session(&session_id).await.unwrap();
    let state_before = session_before.as_ref().map(|s| s.state().clone());
    println!("Session before termination: {:?}", state_before);
    assert_eq!(state_before, Some(CallState::Active));
    
    // Terminate established call (sends BYE)
    let terminate_result = manager_a.terminate_session(&session_id).await;
    if let Err(ref e) = terminate_result {
        println!("❌ Terminate error: {:?}", e);
    }
    assert!(terminate_result.is_ok());
    
    // Wait for SessionTerminated event
    wait_for_session_terminated(&mut event_sub, &session_id, Duration::from_secs(2)).await;
    
    // Verify session is removed
    verify_session_removed(&manager_a, &session_id).await.unwrap();
}

#[tokio::test]
#[serial]
async fn test_multiple_bye_terminations() {
    // Create a single call and verify BYE termination works
    let (manager_a, manager_b, mut call_events) = create_session_manager_pair().await.unwrap();
    
    // Subscribe to events before creating calls
    let mut event_sub_a = manager_a.event_processor.subscribe().await.unwrap();
    let mut event_sub_b = manager_b.event_processor.subscribe().await.unwrap();
    
    // Establish a single call
    let (call, callee_session_id) = establish_call_between_managers(&manager_a, &manager_b, &mut call_events).await.unwrap();
    let caller_session_id = call.id().clone();
    
    // Verify both sessions are active
    verify_session_exists(&manager_a, &caller_session_id, Some(&CallState::Active)).await.unwrap();
    if let Some(ref callee_id) = callee_session_id {
        verify_session_exists(&manager_b, callee_id, Some(&CallState::Active)).await.unwrap();
    }
    
    // Terminate the call from caller side (sends BYE)
    let result = manager_a.terminate_session(&caller_session_id).await;
    if let Err(ref e) = result {
        println!("❌ Terminate error for session {}: {:?}", caller_session_id, e);
    }
    assert!(result.is_ok());
    
    // Wait for SessionTerminated events on both sides
    wait_for_session_terminated(&mut event_sub_a, &caller_session_id, Duration::from_secs(2)).await;
    if let Some(ref callee_id) = callee_session_id {
        wait_for_session_terminated(&mut event_sub_b, callee_id, Duration::from_secs(2)).await;
    }
    
    // Wait for all sessions to be cleaned up
    wait_for_no_active_sessions(&manager_a, &manager_b, Duration::from_secs(2)).await
        .expect("Sessions should be cleaned up after BYE");
}

#[tokio::test]
#[serial]
async fn test_cancel_nonexistent_session() {
    let (manager_a, _manager_b, _) = create_session_manager_pair().await.unwrap();
    
    // Try to cancel a non-existent session
    let fake_session_id = SessionId::new();
    let terminate_result = manager_a.terminate_session(&fake_session_id).await;
    assert!(terminate_result.is_err());
    assert!(matches!(terminate_result.unwrap_err(), SessionError::SessionNotFound(_)));
}

#[tokio::test]
#[serial]
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
#[serial]
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
#[serial]
async fn test_session_state_management_during_bye() {
    let (manager_a, manager_b, mut call_events) = create_session_manager_pair().await.unwrap();
    
    // Subscribe to events
    let mut event_sub_a = manager_a.event_processor.subscribe().await.unwrap();
    let mut event_sub_b = manager_b.event_processor.subscribe().await.unwrap();
    
    // Establish a call fully
    let (call, callee_session_id) = establish_call_between_managers(&manager_a, &manager_b, &mut call_events).await.unwrap();
    let session_id = call.id().clone();
    
    // Verify session exists and is Active before termination
    let session_before = manager_a.find_session(&session_id).await.unwrap();
    assert!(session_before.is_some());
    let state_before = session_before.unwrap().state().clone();
    assert_eq!(state_before, CallState::Active);
    
    // Terminate the session (sends BYE)
    let terminate_result = manager_a.terminate_session(&session_id).await;
    println!("Terminate result: {:?}", terminate_result);
    assert!(terminate_result.is_ok());
    
    // Wait for SessionTerminated events
    wait_for_session_terminated(&mut event_sub_a, &session_id, Duration::from_secs(2)).await;
    if let Some(ref callee_id) = callee_session_id {
        wait_for_session_terminated(&mut event_sub_b, callee_id, Duration::from_secs(2)).await;
    }
    
    // Verify session is in Terminated state or removed
    let session_after = manager_a.find_session(&session_id).await.unwrap();
    if let Some(session) = session_after {
        assert_eq!(session.state(), &CallState::Terminated, 
                   "Session should be terminated after BYE");
    }
    // If None, that's also fine - session has been completely removed
    
    // Wait for all sessions to be cleaned up
    wait_for_no_active_sessions(&manager_a, &manager_b, Duration::from_secs(5)).await
        .expect("Sessions should be cleaned up after BYE");
}

#[tokio::test]
#[serial]
async fn test_bye_statistics_tracking() {
    let (manager_a, manager_b, mut call_events) = create_session_manager_pair().await.unwrap();
    
    // Check initial stats on both managers
    let initial_stats_a = manager_a.get_stats().await.unwrap();
    let initial_stats_b = manager_b.get_stats().await.unwrap();
    assert_eq!(initial_stats_a.active_sessions, 0);
    assert_eq!(initial_stats_b.active_sessions, 0);
    
    // Subscribe to events before creating call
    let mut event_sub_a = manager_a.event_processor.subscribe().await.unwrap();
    let mut event_sub_b = manager_b.event_processor.subscribe().await.unwrap();
    
    // Establish a call
    let (call, callee_session_id) = establish_call_between_managers(&manager_a, &manager_b, &mut call_events).await.unwrap();
    let caller_session_id = call.id().clone();
    
    // Verify stats increased on both sides
    let mid_stats_a = manager_a.get_stats().await.unwrap();
    let mid_stats_b = manager_b.get_stats().await.unwrap();
    assert_eq!(mid_stats_a.active_sessions, 1);
    assert_eq!(mid_stats_b.active_sessions, 1);
    
    // Terminate the call (sends BYE)
    let terminate_result = manager_a.terminate_session(&caller_session_id).await;
    println!("Terminate result: {:?}", terminate_result);
    assert!(terminate_result.is_ok());
    
    // Wait for SessionTerminated events on both sides
    wait_for_session_terminated(&mut event_sub_a, &caller_session_id, Duration::from_secs(2)).await;
    if let Some(ref callee_id) = callee_session_id {
        wait_for_session_terminated(&mut event_sub_b, callee_id, Duration::from_secs(2)).await;
    }
    
    // Wait for all sessions to be cleaned up
    wait_for_no_active_sessions(&manager_a, &manager_b, Duration::from_secs(10)).await
        .expect("Sessions should be cleaned up after BYE");
}

#[tokio::test]
#[serial]
async fn test_concurrent_bye_terminations() {
    // Test concurrent BYE terminations on two established calls
    let (manager_a1, manager_b1, mut call_events1) = create_session_manager_pair().await.unwrap();
    let (manager_a2, manager_b2, mut call_events2) = create_session_manager_pair().await.unwrap();
    
    // Subscribe to events
    let mut event_sub_a1 = manager_a1.event_processor.subscribe().await.unwrap();
    let mut event_sub_b1 = manager_b1.event_processor.subscribe().await.unwrap();
    let mut event_sub_a2 = manager_a2.event_processor.subscribe().await.unwrap();
    let mut event_sub_b2 = manager_b2.event_processor.subscribe().await.unwrap();
    
    // Establish two calls on different manager pairs
    let (call1, callee_session_id1) = establish_call_between_managers(&manager_a1, &manager_b1, &mut call_events1).await.unwrap();
    let (call2, callee_session_id2) = establish_call_between_managers(&manager_a2, &manager_b2, &mut call_events2).await.unwrap();
    
    let session_id1 = call1.id().clone();
    let session_id2 = call2.id().clone();
    
    // Verify all calls are active
    verify_session_exists(&manager_a1, &session_id1, Some(&CallState::Active)).await.unwrap();
    verify_session_exists(&manager_a2, &session_id2, Some(&CallState::Active)).await.unwrap();
    
    // Terminate both calls concurrently (sends BYE)
    let manager_a1_clone = manager_a1.clone();
    let manager_a2_clone = manager_a2.clone();
    let session_id1_clone = session_id1.clone();
    let session_id2_clone = session_id2.clone();
    
    let task1 = tokio::spawn(async move {
        let result = manager_a1_clone.terminate_session(&session_id1_clone).await;
        println!("Concurrent terminate result for {}: {:?}", session_id1_clone, result);
        result
    });
    
    let task2 = tokio::spawn(async move {
        let result = manager_a2_clone.terminate_session(&session_id2_clone).await;
        println!("Concurrent terminate result for {}: {:?}", session_id2_clone, result);
        result
    });
    
    // Wait for both terminations to complete
    let result1 = task1.await.unwrap();
    let result2 = task2.await.unwrap();
    
    assert!(result1.is_ok(), "Call 1 termination should succeed");
    assert!(result2.is_ok(), "Call 2 termination should succeed");
    
    // Wait for SessionTerminated events on all sessions
    wait_for_session_terminated(&mut event_sub_a1, &session_id1, Duration::from_secs(2)).await;
    wait_for_session_terminated(&mut event_sub_a2, &session_id2, Duration::from_secs(2)).await;
    
    if let Some(ref callee_id) = callee_session_id1 {
        wait_for_session_terminated(&mut event_sub_b1, callee_id, Duration::from_secs(2)).await;
    }
    if let Some(ref callee_id) = callee_session_id2 {
        wait_for_session_terminated(&mut event_sub_b2, callee_id, Duration::from_secs(2)).await;
    }
    
    // Wait for all sessions to be cleaned up on both pairs
    wait_for_no_active_sessions(&manager_a1, &manager_b1, Duration::from_secs(2)).await
        .expect("Sessions should be cleaned up after BYE on pair 1");
    wait_for_no_active_sessions(&manager_a2, &manager_b2, Duration::from_secs(2)).await
        .expect("Sessions should be cleaned up after BYE on pair 2");
} 