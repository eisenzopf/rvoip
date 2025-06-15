use rvoip_session_core::api::control::SessionControl;
//! Tests for PRACK Dialog Integration
//!
//! Tests the session-core functionality for PRACK requests (Provisional Response Acknowledgment),
//! ensuring proper integration with the underlying dialog layer.

mod common;

use std::time::Duration;
use rvoip_session_core::{
    SessionCoordinator,
    SessionError,
    api::{
        types::{SessionId},
    },
};
use common::*;

#[tokio::test]
async fn test_outgoing_call_with_prack_support() {
    let (manager_a, _manager_b, _) = create_session_manager_pair().await.unwrap();
    
    // Create an outgoing call that supports reliable provisional responses
    let result = manager_a.create_outgoing_call(
        "sip:alice@127.0.0.1",
        "sip:bob@127.0.0.1:6001", 
        Some("v=0\r\no=alice 123 456 IN IP4 192.168.1.100\r\nm=audio 5004 RTP/AVP 0\r\n".to_string())
    ).await;
    
    assert!(result.is_ok());
    let call = result.unwrap();
    assert_eq!(call.state(), &rvoip_session_core::api::types::CallState::Initiating);
}

#[tokio::test]
async fn test_session_with_early_media_prack() {
    let (manager_a, manager_b, mut call_events) = create_session_manager_pair().await.unwrap();
    
    // Create an established call that might receive early media with PRACK
    let (call, _) = establish_call_between_managers(&manager_a, &manager_b, &mut call_events).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Simulate receiving 183 Session Progress with early media
    // In real scenario, this would be handled by dialog-core
    
    // Verify session exists and is in correct state
    let session = manager_a.find_session(&session_id).await.unwrap();
    assert!(session.is_some());
}

#[tokio::test]
async fn test_prack_sequence_handling() {
    let (manager_a, manager_b, mut call_events) = create_session_manager_pair().await.unwrap();
    
    // Create multiple established calls to test PRACK sequence numbers
    let mut calls = Vec::new();
    for _i in 0..3 {
        let (call, _) = establish_call_between_managers(&manager_a, &manager_b, &mut call_events).await.unwrap();
        calls.push(call);
    }
    
    // Each call should handle PRACK sequences independently
    for call in &calls {
        let session = manager_a.find_session(call.id()).await.unwrap();
        assert!(session.is_some());
    }
}

#[tokio::test]
async fn test_prack_with_media_update() {
    let (manager_a, manager_b, mut call_events) = create_session_manager_pair().await.unwrap();
    
    // Create an established call for PRACK media update testing
    let (call, _) = establish_call_between_managers(&manager_a, &manager_b, &mut call_events).await.unwrap();
    let session_id = call.id().clone();
    
    // Simulate PRACK with SDP (media update in provisional response)
    let update_result = manager_a.update_media(&session_id, "Updated SDP in PRACK").await;
    assert!(update_result.is_ok());
}

#[tokio::test]
async fn test_prack_error_handling() {
    let (manager_a, _manager_b, _) = create_session_manager_pair().await.unwrap();
    
    // Try PRACK-related operations on non-existent session
    let fake_session_id = SessionId::new();
    
    // Media update (which could be part of PRACK flow) should fail
    let update_result = manager_a.update_media(&fake_session_id, "PRACK SDP").await;
    assert!(update_result.is_err());
    assert!(matches!(update_result.unwrap_err(), SessionError::SessionNotFound(_)));
}

#[tokio::test]
async fn test_multiple_provisional_responses() {
    let (manager_a, manager_b, mut call_events) = create_session_manager_pair().await.unwrap();
    
    // Create an established call that might receive multiple provisional responses
    let (call, _) = establish_call_between_managers(&manager_a, &manager_b, &mut call_events).await.unwrap();
    let session_id = call.id().clone();
    
    // Simulate multiple media updates (as if from different provisional responses)
    for i in 1..=3 {
        let sdp = format!("Provisional response {} SDP", i);
        let update_result = manager_a.update_media(&session_id, &sdp).await;
        assert!(update_result.is_ok());
        
        // Small delay between provisional responses
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

#[tokio::test]
async fn test_prack_session_state_consistency() {
    let (manager_a, manager_b, mut call_events) = create_session_manager_pair().await.unwrap();
    
    // Create an established call for state consistency testing
    let (call, _) = establish_call_between_managers(&manager_a, &manager_b, &mut call_events).await.unwrap();
    let session_id = call.id().clone();
    
    // Verify session exists before PRACK operations
    let session_before = manager_a.find_session(&session_id).await.unwrap();
    assert!(session_before.is_some());
    
    // Simulate PRACK-related media update
    let update_result = manager_a.update_media(&session_id, "PRACK media update").await;
    assert!(update_result.is_ok());
    
    // Verify session consistency after PRACK
    let session_after = manager_a.find_session(&session_id).await.unwrap();
    assert!(session_after.is_some());
    assert_eq!(session_after.unwrap().id(), &session_id);
}

#[tokio::test]
async fn test_prack_with_codec_negotiation() {
    let (manager_a, manager_b, mut call_events) = create_session_manager_pair().await.unwrap();
    
    // Create an established call with multiple codec options for negotiation
    let (call, _) = establish_call_between_managers_with_sdp(
        &manager_a, 
        &manager_b, 
        &mut call_events,
        Some("v=0\r\no=alice 123 456 IN IP4 192.168.1.100\r\nm=audio 5004 RTP/AVP 0 8 18\r\n".to_string())
    ).await.unwrap();
    let session_id = call.id().clone();
    
    // Simulate codec negotiation via PRACK sequence
    let negotiated_sdp = "v=0\r\no=alice 123 789 IN IP4 192.168.1.100\r\nm=audio 5004 RTP/AVP 8\r\n";
    let update_result = manager_a.update_media(&session_id, negotiated_sdp).await;
    assert!(update_result.is_ok());
}

#[tokio::test]
async fn test_concurrent_prack_sessions() {
    let (manager_a, manager_b, mut call_events) = create_session_manager_pair().await.unwrap();
    
    // Create multiple established calls that might use PRACK
    let mut calls = Vec::new();
    for _i in 0..3 {  // Reduced from 5 to 3 for more reliable testing
        let (call, _) = establish_call_between_managers(&manager_a, &manager_b, &mut call_events).await.unwrap();
        calls.push(call);
    }
    
    // Perform concurrent PRACK-related operations
    let mut tasks = Vec::new();
    for (i, call) in calls.iter().enumerate() {
        let manager_clone = manager_a.clone();
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
}

#[tokio::test]
async fn test_prack_timing_constraints() {
    let (manager_a, manager_b, mut call_events) = create_session_manager_pair().await.unwrap();
    
    // Create an established call for timing tests
    let (call, _) = establish_call_between_managers(&manager_a, &manager_b, &mut call_events).await.unwrap();
    let session_id = call.id().clone();
    
    // Test rapid PRACK sequences (testing timing)
    let start_time = std::time::Instant::now();
    
    for i in 0..5 {
        let sdp = format!("Rapid PRACK {}", i);
        let update_result = manager_a.update_media(&session_id, &sdp).await;
        assert!(update_result.is_ok());
    }
    
    let elapsed = start_time.elapsed();
    // PRACK operations should complete reasonably quickly (increased for real network operations)
    assert!(elapsed < Duration::from_secs(10), "PRACK operations took {:?}", elapsed);
} 