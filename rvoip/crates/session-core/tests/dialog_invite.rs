use rvoip_session_core::api::control::SessionControl;
// Tests for INVITE Dialog Integration
//
// Tests the session-core functionality for INVITE dialogs (voice/video calls),
// ensuring proper integration with the underlying dialog layer.
// These tests use real session events from the infra-common zero-copy event system.

mod common;

use std::sync::Arc;
use std::time::Duration;
use rvoip_session_core::{
    SessionCoordinator,
    SessionError,
    api::{
        types::{CallState, SessionId, CallSession, CallDecision},
        handlers::CallHandler,
    },
};
use common::*;

#[tokio::test]
async fn test_outgoing_call_creation() {
    let (manager_a, manager_b, _) = create_session_manager_pair().await.unwrap();
    
    // Create an outgoing call (INVITE dialog)
    let result = manager_a.create_outgoing_call(
        "sip:alice@localhost",
        "sip:bob@localhost", 
        Some("v=0\r\no=alice 123 456 IN IP4 127.0.0.1\r\n...".to_string())
    ).await;
    
    assert!(result.is_ok());
    let call = result.unwrap();
    assert_eq!(call.state(), &CallState::Initiating);
    assert_eq!(call.from, "sip:alice@localhost");
    assert_eq!(call.to, "sip:bob@localhost");
    
    // Clean up
    cleanup_managers(vec![manager_a, manager_b]).await.unwrap();
}

#[tokio::test]
async fn test_outgoing_call_without_sdp() {
    let (manager_a, manager_b, _) = create_session_manager_pair().await.unwrap();
    
    // Create call without SDP offer
    let result = manager_a.create_outgoing_call(
        "sip:alice@localhost",
        "sip:bob@localhost",
        None
    ).await;
    
    assert!(result.is_ok());
    let call = result.unwrap();
    assert_eq!(call.state(), &CallState::Initiating);
    
    cleanup_managers(vec![manager_a, manager_b]).await.unwrap();
}

#[tokio::test]
async fn test_call_establishment_between_managers() {
    let (manager_a, manager_b, mut call_events) = create_session_manager_pair().await.unwrap();
    
    let (call, callee_session_id) = establish_call_between_managers(&manager_a, &manager_b, &mut call_events).await.unwrap();
    
    // Verify call was created with dynamic URIs based on actual bound addresses
    let caller_addr = manager_a.get_bound_address();
    let callee_addr = manager_b.get_bound_address();
    let expected_from = format!("sip:alice@{}", caller_addr.ip());
    let expected_to = format!("sip:bob@{}", callee_addr);
    
    assert_eq!(call.from, expected_from);
    assert_eq!(call.to, expected_to);
    
    // Check that session exists (should be Active after INVITE/200OK/ACK)
    verify_session_exists(&manager_a, call.id(), Some(&CallState::Active)).await.unwrap();
    
    // Verify callee received the call
    if let Some(_callee_id) = callee_session_id {
        println!("✓ Callee received incoming call");
    } else {
        println!("⚠ Callee did not receive call within timeout");
    }
    
    cleanup_managers(vec![manager_a, manager_b]).await.unwrap();
}

#[tokio::test]
async fn test_session_hold_and_resume_on_established_call() {
    let (manager_a, manager_b, mut call_events) = create_session_manager_pair().await.unwrap();
    
    let (call, _) = establish_call_between_managers(&manager_a, &manager_b, &mut call_events).await.unwrap();
    let session_id = call.id().clone();
    
    // Subscribe to events for this test
    // Event processor not available - skipping event subscription
    
    // Test hold operation
    let hold_result = manager_a.hold_session(&session_id).await;
    println!("Hold result: {:?}", hold_result);
    
    if hold_result.is_ok() {
        // Wait for state change event
        // if let Some((old_state, new_state)) = wait_for_state_change(&mut events, &session_id, Duration::from_secs(1)).await {
        //     println!("Hold state change: {:?} -> {:?}", old_state, new_state);
        // }
        println!("Hold operation succeeded");
        
        // Test resume operation
        let resume_result = manager_a.resume_session(&session_id).await;
        println!("Resume result: {:?}", resume_result);
        
        if resume_result.is_ok() {
            // Wait for another state change event
            // if let Some((old_state, new_state)) = wait_for_state_change(&mut events, &session_id, Duration::from_secs(1)).await {
            //     println!("Resume state change: {:?} -> {:?}", old_state, new_state);
            // }
            println!("Resume operation succeeded");
        }
    }
    
    cleanup_managers(vec![manager_a, manager_b]).await.unwrap();
}

#[tokio::test]
async fn test_session_transfer_on_established_call() {
    let (manager_a, manager_b, mut call_events) = create_session_manager_pair().await.unwrap();
    
    let (call, _) = establish_call_between_managers(&manager_a, &manager_b, &mut call_events).await.unwrap();
    let session_id = call.id().clone();
    
    // Subscribe to events for this test
    // Event processor not available - skipping event subscription
    
    // Test transfer operation
    let transfer_result = manager_a.transfer_session(&session_id, "sip:charlie@localhost").await;
    println!("Transfer result: {:?}", transfer_result);
    
    if transfer_result.is_ok() {
        // Wait for state change event
        // if let Some((old_state, new_state)) = wait_for_state_change(&mut events, &session_id, Duration::from_secs(1)).await {
        //     println!("Transfer state change: {:?} -> {:?}", old_state, new_state);
        // }
        println!("Transfer operation succeeded");
    }
    
    cleanup_managers(vec![manager_a, manager_b]).await.unwrap();
}

#[tokio::test]
async fn test_session_dtmf_on_established_call() {
    let (manager_a, manager_b, mut call_events) = create_session_manager_pair().await.unwrap();
    
    let (call, _) = establish_call_between_managers(&manager_a, &manager_b, &mut call_events).await.unwrap();
    let session_id = call.id().clone();
    
    // Test DTMF sending
    let dtmf_result = manager_a.send_dtmf(&session_id, "123").await;
    println!("DTMF result: {:?}", dtmf_result);
    
    // Test multiple DTMF digits
    let dtmf_result = manager_a.send_dtmf(&session_id, "*#0987654321").await;
    println!("Multi-DTMF result: {:?}", dtmf_result);
    
    cleanup_managers(vec![manager_a, manager_b]).await.unwrap();
}

#[tokio::test]
async fn test_session_media_update_on_established_call() {
    let (manager_a, manager_b, mut call_events) = create_session_manager_pair().await.unwrap();
    
    let (call, _) = establish_call_between_managers(&manager_a, &manager_b, &mut call_events).await.unwrap();
    let session_id = call.id().clone();
    
    // Test media update
    let media_result = manager_a.update_media(&session_id, "v=0\r\no=alice 456 789 IN IP4 127.0.0.1\r\n...").await;
    println!("Media update result: {:?}", media_result);
    
    cleanup_managers(vec![manager_a, manager_b]).await.unwrap();
}

#[tokio::test]
async fn test_session_termination_on_established_call() {
    let (manager_a, manager_b, mut call_events) = create_session_manager_pair().await.unwrap();
    
    let (call, _) = establish_call_between_managers(&manager_a, &manager_b, &mut call_events).await.unwrap();
    let session_id = call.id().clone();
    
    // Subscribe to events for this test
    // Event processor not available - skipping event subscription
    
    // Verify session exists
    verify_session_exists(&manager_a, &session_id, None).await.unwrap();
    
    // Test termination
    let terminate_result = manager_a.terminate_session(&session_id).await;
    println!("Terminate result: {:?}", terminate_result);
    
    if terminate_result.is_ok() {
        // Wait for session terminated event
        // if let Some(reason) = wait_for_session_terminated(&mut events, &session_id, Duration::from_secs(2)).await {
        //     println!("Session terminated with reason: {}", reason);
        // }
        println!("Session terminated successfully");
        
        // Wait for the session to transition to Terminated state
        // Let's wait longer and check session state multiple times
        for i in 1..=10 {
            tokio::time::sleep(Duration::from_millis(200)).await;
            
            // Verify session is in Terminated state (not removed, as sessions stay in registry)
            let session = manager_a.find_session(&session_id).await.unwrap();
            if let Some(session) = session {
                println!("Attempt {}: Session state is {:?}", i, session.state());
                if session.state() == &CallState::Terminated {
                    println!("✓ Session successfully terminated");
                    return; // Test passes
                }
            } else {
                panic!("Session was removed from registry, expected it to remain in Terminated state");
            }
        }
        
        // If we get here, the session never transitioned to Terminated
        let session = manager_a.find_session(&session_id).await.unwrap();
        if let Some(session) = session {
            panic!("Session failed to transition to Terminated state after 2 seconds. Current state: {:?}", session.state());
        }
    }
    
    cleanup_managers(vec![manager_a, manager_b]).await.unwrap();
}

#[tokio::test]
async fn test_session_operations_on_nonexistent_session() {
    let (manager_a, _manager_b, _) = create_session_manager_pair().await.unwrap();
    
    let fake_session_id = SessionId::new();
    
    // Test termination on non-existent session
    let terminate_result = manager_a.terminate_session(&fake_session_id).await;
    assert!(terminate_result.is_err());
    
    // Note: hold, resume, and transfer are currently stub implementations that always return Ok()
    // Once implemented, they should return errors for non-existent sessions
    
    // Test hold on non-existent session (currently returns Ok due to stub implementation)
    let hold_result = manager_a.hold_session(&fake_session_id).await;
    println!("Hold result (stub): {:?}", hold_result);
    
    // The hold_session method now properly returns an error for non-existent sessions
    assert!(hold_result.is_err(), "Hold should return an error for non-existent session");
    match hold_result.unwrap_err() {
        SessionError::SessionNotFound(_) => {
            println!("Got expected SessionNotFound error for hold on non-existent session");
        }
        other => {
            panic!("Expected SessionNotFound error for hold, got: {:?}", other);
        }
    }
    
    // Test resume on non-existent session (currently returns Ok due to stub implementation)
    let resume_result = manager_a.resume_session(&fake_session_id).await;
    println!("Resume result (stub): {:?}", resume_result);
    
    // The resume_session method now properly returns an error for non-existent sessions
    assert!(resume_result.is_err(), "Resume should return an error for non-existent session");
    match resume_result.unwrap_err() {
        SessionError::SessionNotFound(_) => {
            println!("Got expected SessionNotFound error for resume on non-existent session");
        }
        other => {
            panic!("Expected SessionNotFound error for resume, got: {:?}", other);
        }
    }
    
    // Test transfer on non-existent session (currently returns Ok due to stub implementation)
    let transfer_result = manager_a.transfer_session(&fake_session_id, "sip:other@localhost").await;
    println!("Transfer result (stub): {:?}", transfer_result);
    
    // The transfer_session method now properly returns an error for non-existent sessions
    assert!(transfer_result.is_err(), "Transfer should return an error for non-existent session");
    match transfer_result.unwrap_err() {
        SessionError::SessionNotFound(_) => {
            println!("Got expected SessionNotFound error for transfer on non-existent session");
        }
        other => {
            panic!("Expected SessionNotFound error for transfer, got: {:?}", other);
        }
    }
    
    // Test DTMF on non-existent session (this is properly implemented and should fail)
    let dtmf_result = manager_a.send_dtmf(&fake_session_id, "123").await;
    println!("DTMF result: {:?}", dtmf_result);
    assert!(dtmf_result.is_err());
    
    match dtmf_result.unwrap_err() {
        SessionError::SessionNotFound(_) => {
            println!("Got expected SessionNotFound error for DTMF on non-existent session");
        }
        other => {
            panic!("Expected SessionNotFound error for DTMF, got: {:?}", other);
        }
    }
}

#[tokio::test]
async fn test_multiple_concurrent_calls() {
    let (manager_a, manager_b, _) = create_session_manager_pair().await.unwrap();
    
    // Create multiple outgoing calls concurrently
    let mut calls = Vec::new();
    
    for i in 0..3 { // Reduced number for more reliable testing
        let call = manager_a.create_outgoing_call(
            &format!("sip:caller{}@localhost", i),
            &format!("sip:target{}@localhost", i),
            Some(format!("v=0\r\no=caller{} 123 456 IN IP4 127.0.0.1\r\n...", i))
        ).await.unwrap();
        calls.push(call);
    }
    
    // Verify all calls were created
    assert_eq!(calls.len(), 3);
    
    // Check that all sessions are tracked
    let stats = manager_a.get_stats().await.unwrap();
    assert_eq!(stats.active_sessions, 3);
    
    cleanup_managers(vec![manager_a, manager_b]).await.unwrap();
}

#[tokio::test]
async fn test_session_manager_with_reject_handler() {
    let (manager_a, manager_b) = create_session_manager_pair_with_handlers(
        Arc::new(common::media_test_utils::TestCallHandler::new(true)),
        Arc::new(common::media_test_utils::TestCallHandler::new(false)), // RejectHandler
    ).await.unwrap();
    
    // Subscribe to events
    // Event processor not available - skipping event subscription
    
    // Create outgoing call (should still work)
    let call = manager_a.create_outgoing_call(
        "sip:alice@localhost",
        "sip:bob@localhost",
        Some("SDP offer".to_string())
    ).await.unwrap();
    
    assert_eq!(call.state(), &CallState::Initiating);
    
    // Wait for potential state change (could go to Failed due to rejection)
    // if let Some((old_state, new_state)) = wait_for_state_change(&mut events, call.id(), Duration::from_secs(2)).await {
    //     println!("Call with reject handler: {:?} -> {:?}", old_state, new_state);
    // } else {
    //     println!("Call with reject handler: Initiating -> (no change)");
    // }
    println!("Call created with reject handler");
    
    cleanup_managers(vec![manager_a, manager_b]).await.unwrap();
}

#[tokio::test]
async fn test_session_stats_tracking() {
    let (manager_a, manager_b, _) = create_session_manager_pair().await.unwrap();
    
    // Check initial stats
    let initial_stats = manager_a.get_stats().await.unwrap();
    assert_eq!(initial_stats.active_sessions, 0);
    
    // Create some calls
    let call1 = manager_a.create_outgoing_call(
        "sip:alice@localhost",
        "sip:bob@localhost",
        Some("SDP 1".to_string())
    ).await.unwrap();
    
    let call2 = manager_a.create_outgoing_call(
        "sip:charlie@localhost",
        "sip:david@localhost",
        Some("SDP 2".to_string())
    ).await.unwrap();
    
    // Check updated stats
    let updated_stats = manager_a.get_stats().await.unwrap();
    assert_eq!(updated_stats.active_sessions, 2);
    
    // Verify sessions can be found
    verify_session_exists(&manager_a, call1.id(), None).await.unwrap();
    verify_session_exists(&manager_a, call2.id(), None).await.unwrap();
    
    cleanup_managers(vec![manager_a, manager_b]).await.unwrap();
} 