//! Tests for Call Transfer Functionality
//!
//! Tests the session-core functionality for call transfers,
//! ensuring proper integration with the underlying dialog layer.

use std::sync::Arc;
use std::time::Duration;
use rvoip_session_core::{
    SessionManager,
    SessionError,
    api::{
        types::{SessionId, IncomingCall, CallSession, CallDecision},
        handlers::CallHandler,
        builder::SessionManagerBuilder,
    },
};

/// Handler for transfer testing
#[derive(Debug)]
struct TransferTestHandler;

#[async_trait::async_trait]
impl CallHandler for TransferTestHandler {
    async fn on_incoming_call(&self, _call: IncomingCall) -> CallDecision {
        CallDecision::Accept
    }

    async fn on_call_ended(&self, call: CallSession, reason: &str) {
        tracing::info!("Transfer test call {} ended: {}", call.id(), reason);
    }
}

/// Create a test session manager for transfer testing
async fn create_transfer_test_manager() -> Result<Arc<SessionManager>, SessionError> {
    let handler = Arc::new(TransferTestHandler);
    
    SessionManagerBuilder::new()
        .with_sip_bind_address("127.0.0.1")
        .with_sip_port(0)
        .with_from_uri("sip:transfer@localhost")
        .with_handler(handler)
        .build()
        .await
}

#[tokio::test]
async fn test_basic_call_transfer() {
    let manager = create_transfer_test_manager().await.unwrap();
    manager.start().await.unwrap();
    
    // Create an outgoing call
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("SDP offer".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Test transfer operation
    let transfer_result = manager.transfer_session(&session_id, "sip:charlie@example.com").await;
    assert!(transfer_result.is_ok());
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_transfer_nonexistent_session() {
    let manager = create_transfer_test_manager().await.unwrap();
    manager.start().await.unwrap();
    
    let fake_session_id = SessionId::new();
    let transfer_result = manager.transfer_session(&fake_session_id, "sip:target@example.com").await;
    assert!(transfer_result.is_err());
    assert!(matches!(transfer_result.unwrap_err(), SessionError::SessionNotFound(_)));
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_transfer_to_various_targets() {
    let manager = create_transfer_test_manager().await.unwrap();
    manager.start().await.unwrap();
    
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("SDP offer".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Test transfers to different types of targets
    let transfer_targets = vec![
        "sip:charlie@example.com",
        "sip:david@another-domain.com",
        "sip:1234@pbx.company.com",
        "sip:conference@meetings.example.com",
        "sip:voicemail@vm.example.com",
    ];
    
    for target in transfer_targets {
        let transfer_result = manager.transfer_session(&session_id, target).await;
        assert!(transfer_result.is_ok(), "Transfer to '{}' should succeed", target);
    }
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_multiple_concurrent_transfers() {
    let manager = create_transfer_test_manager().await.unwrap();
    manager.start().await.unwrap();
    
    // Create multiple calls
    let mut sessions = Vec::new();
    for i in 0..5 {
        let call = manager.create_outgoing_call(
            &format!("sip:caller{}@example.com", i),
            &format!("sip:target{}@example.com", i),
            Some(format!("SDP for call {}", i))
        ).await.unwrap();
        sessions.push(call.id().clone());
    }
    
    // Transfer each call to a different target
    for (i, session_id) in sessions.iter().enumerate() {
        let transfer_result = manager.transfer_session(
            session_id, 
            &format!("sip:transfer_target_{}@example.com", i)
        ).await;
        assert!(transfer_result.is_ok(), "Transfer of session {} should succeed", i);
    }
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_transfer_after_other_operations() {
    let manager = create_transfer_test_manager().await.unwrap();
    manager.start().await.unwrap();
    
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("SDP offer".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Perform operations before transfer
    manager.hold_session(&session_id).await.unwrap();
    manager.resume_session(&session_id).await.unwrap();
    manager.send_dtmf(&session_id, "123").await.unwrap();
    manager.update_media(&session_id, "Updated SDP").await.unwrap();
    
    // Now try transfer
    let transfer_result = manager.transfer_session(&session_id, "sip:charlie@example.com").await;
    assert!(transfer_result.is_ok());
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_rapid_transfer_requests() {
    let manager = create_transfer_test_manager().await.unwrap();
    manager.start().await.unwrap();
    
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("SDP offer".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Send multiple rapid transfer requests
    for i in 0..10 {
        let transfer_result = manager.transfer_session(
            &session_id, 
            &format!("sip:rapid_target_{}@example.com", i)
        ).await;
        assert!(transfer_result.is_ok(), "Rapid transfer {} should succeed", i);
        
        // Very small delay
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_transfer_with_session_stats() {
    let manager = create_transfer_test_manager().await.unwrap();
    manager.start().await.unwrap();
    
    // Create a call
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("SDP offer".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Check stats before transfer
    let stats_before = manager.get_stats().await.unwrap();
    assert_eq!(stats_before.active_sessions, 1);
    
    // Perform transfer
    let transfer_result = manager.transfer_session(&session_id, "sip:charlie@example.com").await;
    assert!(transfer_result.is_ok());
    
    // Check stats after transfer (session should still exist until termination)
    let stats_after = manager.get_stats().await.unwrap();
    // The behavior here depends on implementation - transfer might or might not affect active session count
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_transfer_then_terminate() {
    let manager = create_transfer_test_manager().await.unwrap();
    manager.start().await.unwrap();
    
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("SDP offer".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Transfer the call
    let transfer_result = manager.transfer_session(&session_id, "sip:charlie@example.com").await;
    assert!(transfer_result.is_ok());
    
    // Then terminate it
    let terminate_result = manager.terminate_session(&session_id).await;
    assert!(terminate_result.is_ok());
    
    // Wait for cleanup
    tokio::time::sleep(Duration::from_millis(50)).await;
    
    // Verify session is cleaned up
    let session_after = manager.find_session(&session_id).await.unwrap();
    assert!(session_after.is_none());
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_transfer_edge_cases() {
    let manager = create_transfer_test_manager().await.unwrap();
    manager.start().await.unwrap();
    
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("SDP offer".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Test transfer to same URI as current target
    let transfer_result = manager.transfer_session(&session_id, "sip:bob@example.com").await;
    assert!(transfer_result.is_ok(), "Transfer to same target should be handled gracefully");
    
    // Test transfer to same URI as caller
    let transfer_result = manager.transfer_session(&session_id, "sip:alice@example.com").await;
    assert!(transfer_result.is_ok(), "Transfer to caller should be handled gracefully");
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_transfer_stress_test() {
    let manager = create_transfer_test_manager().await.unwrap();
    manager.start().await.unwrap();
    
    // Create many calls and transfer them concurrently
    let mut sessions = Vec::new();
    
    for i in 0..20 {
        let call = manager.create_outgoing_call(
            &format!("sip:stress_caller_{}@example.com", i),
            &format!("sip:stress_target_{}@example.com", i),
            Some(format!("Stress test SDP {}", i))
        ).await.unwrap();
        sessions.push(call.id().clone());
    }
    
    // Transfer all calls concurrently
    let mut handles = Vec::new();
    for (i, session_id) in sessions.iter().enumerate() {
        let manager_clone: Arc<SessionManager> = Arc::clone(&manager);
        let session_id_clone = session_id.clone();
        let handle = tokio::spawn(async move {
            manager_clone.transfer_session(
                &session_id_clone, 
                &format!("sip:stress_transfer_{}@example.com", i)
            ).await
        });
        handles.push(handle);
    }
    
    // Wait for all transfers to complete
    for handle in handles {
        let result = handle.await.unwrap();
        assert!(result.is_ok());
    }
    
    manager.stop().await.unwrap();
} 