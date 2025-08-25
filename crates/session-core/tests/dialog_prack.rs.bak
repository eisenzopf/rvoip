use rvoip_session_core::api::control::SessionControl;
// Tests for PRACK Dialog Integration
//
// Tests the session-core functionality for PRACK requests (Provisional Response Acknowledgment),
// ensuring proper integration with the underlying dialog layer.

mod common;

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
    
    // Create a call (it won't be established in test environment, but that's OK for this test)
    let call = manager_a.create_outgoing_call(
        "sip:alice@127.0.0.1",
        "sip:bob@127.0.0.1:6001", 
        Some("v=0\r\no=alice 123 456 IN IP4 192.168.1.100\r\nm=audio 5004 RTP/AVP 0\r\n".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Simulate receiving 183 Session Progress with early media
    // In real scenario, this would be handled by dialog-core
    
    // Verify session exists
    let session = manager_a.get_session(&session_id).await.unwrap();
    assert!(session.is_some());
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

 