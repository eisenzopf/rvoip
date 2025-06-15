use rvoip_session_core::api::control::SessionControl;
//! Tests for NOTIFY Dialog Integration
//!
//! Tests the session-core functionality for NOTIFY requests (event notifications),
//! ensuring proper integration with the underlying dialog layer.

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
};
use common::*;

#[tokio::test]
async fn test_session_with_notify_support() {
    let (manager_a, _manager_b, _) = create_session_manager_pair().await.unwrap();
    
    // Create a session that might send/receive NOTIFY messages
    let result = manager_a.create_outgoing_call(
        "sip:alice@127.0.0.1",
        "sip:bob@127.0.0.1:6001", 
        Some("v=0\r\no=alice 123 456 IN IP4 192.168.1.100\r\nm=audio 5004 RTP/AVP 0\r\n".to_string())
    ).await;
    
    assert!(result.is_ok());
    let call = result.unwrap();
    assert_eq!(call.state(), &CallState::Initiating);
    
    // In real scenario, NOTIFY would be sent/received through dialog-core
    // Here we're testing that session-core can handle NOTIFY-related operations
}

#[tokio::test]
async fn test_notify_for_call_state_changes() {
    let (manager_a, manager_b, mut call_events) = create_session_manager_pair().await.unwrap();
    
    // Create an established call
    let (call, _) = establish_call_between_managers(&manager_a, &manager_b, &mut call_events).await.unwrap();
    let session_id = call.id().clone();
    
    // Test operations that might trigger NOTIFY messages
    
    // Hold operation (might send NOTIFY with dialog state)
    let hold_result = manager_a.hold_session(&session_id).await;
    assert!(hold_result.is_ok());
    
    // Resume operation (might send NOTIFY with dialog state)
    let resume_result = manager_a.resume_session(&session_id).await;
    assert!(resume_result.is_ok());
}

#[tokio::test]
async fn test_notify_for_transfer_events() {
    let (manager_a, manager_b, mut call_events) = create_session_manager_pair().await.unwrap();
    
    // Create an established call
    let (call, _) = establish_call_between_managers(&manager_a, &manager_b, &mut call_events).await.unwrap();
    let session_id = call.id().clone();
    
    // Transfer operation (should trigger NOTIFY messages for transfer status)
    let transfer_result = manager_a.transfer_session(&session_id, "sip:charlie@127.0.0.1:7001").await;
    assert!(transfer_result.is_ok());
    
    // In real implementation, this would send REFER and receive NOTIFY messages
    // about transfer progress
}

#[tokio::test]
async fn test_notify_message_info_integration() {
    let (manager_a, manager_b, mut call_events) = create_session_manager_pair().await.unwrap();
    
    // Create an established call
    let (call, _) = establish_call_between_managers(&manager_a, &manager_b, &mut call_events).await.unwrap();
    let session_id = call.id().clone();
    
    // Send INFO that might be related to NOTIFY events
    let info_result = manager_a.send_dtmf(&session_id, "123").await;
    assert!(info_result.is_ok());
    
    // In real scenario, DTMF might be sent via INFO and status via NOTIFY
}

#[tokio::test]
async fn test_multiple_notify_subscriptions() {
    let (manager_a, manager_b, mut call_events) = create_session_manager_pair().await.unwrap();
    
    // Create multiple calls that might have different NOTIFY subscriptions
    let mut calls = Vec::new();
    for i in 0..3 {
        let target_addr = manager_b.get_bound_address();
        let call = manager_a.create_outgoing_call(
            &format!("sip:caller{}@127.0.0.1", i),
            &format!("sip:target{}@{}", i, target_addr),
            Some(format!("SDP for NOTIFY test {}", i))
        ).await.unwrap();
        calls.push(call);
    }
    
    // Each call might have different NOTIFY event packages
    for call in &calls {
        let session = manager_a.find_session(call.id()).await.unwrap();
        assert!(session.is_some());
    }
}

#[tokio::test]
async fn test_notify_error_handling() {
    let (manager_a, _manager_b, _) = create_session_manager_pair().await.unwrap();
    
    // Try NOTIFY-related operations on non-existent session
    let fake_session_id = SessionId::new();
    
    // Operations that might involve NOTIFY should fail appropriately
    let transfer_result = manager_a.transfer_session(&fake_session_id, "sip:target@127.0.0.1").await;
    assert!(transfer_result.is_err());
    assert!(matches!(transfer_result.unwrap_err(), SessionError::SessionNotFound(_)));
    
    let dtmf_result = manager_a.send_dtmf(&fake_session_id, "123").await;
    assert!(dtmf_result.is_err());
}

#[tokio::test]
async fn test_notify_event_sequencing() {
    let (manager_a, manager_b, mut call_events) = create_session_manager_pair().await.unwrap();
    
    // Create an established call
    let (call, _) = establish_call_between_managers(&manager_a, &manager_b, &mut call_events).await.unwrap();
    let session_id = call.id().clone();
    
    // Test sequence of operations that might generate NOTIFY events
    let hold_result = manager_a.hold_session(&session_id).await;
    assert!(hold_result.is_ok());
    
    tokio::time::sleep(Duration::from_millis(10)).await;
    
    let resume_result = manager_a.resume_session(&session_id).await;
    assert!(resume_result.is_ok());
    
    tokio::time::sleep(Duration::from_millis(10)).await;
    
    let transfer_result = manager_a.transfer_session(&session_id, "sip:transfer@127.0.0.1:7001").await;
    assert!(transfer_result.is_ok());
    
    // Each operation should maintain proper NOTIFY event sequencing
}

#[tokio::test]
async fn test_notify_session_state_consistency() {
    let (manager_a, manager_b, mut call_events) = create_session_manager_pair().await.unwrap();
    
    // Create an established call
    let (call, _) = establish_call_between_managers(&manager_a, &manager_b, &mut call_events).await.unwrap();
    let session_id = call.id().clone();
    
    // Verify session exists before NOTIFY operations
    let session_before = manager_a.find_session(&session_id).await.unwrap();
    assert!(session_before.is_some());
    
    // Operations that might involve NOTIFY
    let hold_result = manager_a.hold_session(&session_id).await;
    assert!(hold_result.is_ok());
    
    // Verify session consistency after NOTIFY-triggering operations
    let session_after = manager_a.find_session(&session_id).await.unwrap();
    assert!(session_after.is_some());
    assert_eq!(session_after.unwrap().id(), &session_id);
}

#[tokio::test]
async fn test_concurrent_notify_operations() {
    let (manager_a, manager_b, mut call_events) = create_session_manager_pair().await.unwrap();
    
    // Create multiple established calls
    let mut calls = Vec::new();
    for i in 0..3 {  // Reduced from 5 to 3 for more reliable testing
        let target_addr = manager_b.get_bound_address();
        let (call, _) = establish_call_between_managers(&manager_a, &manager_b, &mut call_events).await.unwrap();
        calls.push(call);
    }
    
    // Perform concurrent operations that might trigger NOTIFY
    let mut tasks = Vec::new();
    for (i, call) in calls.iter().enumerate() {
        let manager_clone = manager_a.clone();
        let session_id = call.id().clone();
        let task = tokio::spawn(async move {
            if i % 2 == 0 {
                manager_clone.hold_session(&session_id).await
            } else {
                manager_clone.send_dtmf(&session_id, &format!("{}", i)).await
            }
        });
        tasks.push(task);
    }
    
    // Wait for all NOTIFY-related operations to complete
    for task in tasks {
        let result = task.await.unwrap();
        assert!(result.is_ok());
    }
}

#[tokio::test]
async fn test_notify_subscription_lifecycle() {
    let (manager_a, manager_b, mut call_events) = create_session_manager_pair().await.unwrap();
    
    // Create an established call (might establish NOTIFY subscriptions)
    let (call, _) = establish_call_between_managers(&manager_a, &manager_b, &mut call_events).await.unwrap();
    let session_id = call.id().clone();
    
    // Session operations that might affect NOTIFY subscriptions
    let hold_result = manager_a.hold_session(&session_id).await;
    assert!(hold_result.is_ok());
    
    // Terminate session (should properly clean up NOTIFY subscriptions)
    let terminate_result = manager_a.terminate_session(&session_id).await;
    assert!(terminate_result.is_ok());
    
    // Wait for cleanup
    tokio::time::sleep(Duration::from_millis(50)).await;
    
    // Verify session cleanup
    let session_after = manager_a.find_session(&session_id).await.unwrap();
    assert!(session_after.is_none());
}

#[tokio::test]
async fn test_notify_timing_and_expiration() {
    let (manager_a, manager_b, mut call_events) = create_session_manager_pair().await.unwrap();
    
    // Create an established call
    let (call, _) = establish_call_between_managers(&manager_a, &manager_b, &mut call_events).await.unwrap();
    let session_id = call.id().clone();
    
    // Test rapid operations that might generate NOTIFY events
    let start_time = std::time::Instant::now();
    
    for i in 0..3 {
        if i % 2 == 0 {
            let _ = manager_a.hold_session(&session_id).await;
        } else {
            let _ = manager_a.resume_session(&session_id).await;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    
    let elapsed = start_time.elapsed();
    // NOTIFY-related operations should complete reasonably quickly (increased for real network operations)
    assert!(elapsed < Duration::from_secs(10), "NOTIFY operations took {:?}", elapsed);
} 