//! Tests for Session Manager Subscription Features
//!
//! Tests the session-core functionality for subscription-related operations,
//! ensuring proper behavior and integration.

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

/// Handler for subscription testing
#[derive(Debug)]
struct SubscriptionTestHandler {
    behavior: HandlerBehavior,
}

#[derive(Debug, Clone)]
enum HandlerBehavior {
    AcceptAll,
    RejectAll,
    AcceptSelective,
}

impl SubscriptionTestHandler {
    fn new(behavior: HandlerBehavior) -> Self {
        Self { behavior }
    }
}

#[async_trait::async_trait]
impl CallHandler for SubscriptionTestHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        match self.behavior {
            HandlerBehavior::AcceptAll => CallDecision::Accept,
            HandlerBehavior::RejectAll => CallDecision::Reject("Test rejection".to_string()),
            HandlerBehavior::AcceptSelective => {
                // Accept calls with specific patterns for testing
                if call.from.contains("accepted") {
                    CallDecision::Accept
                } else {
                    CallDecision::Reject("Selective rejection".to_string())
                }
            }
        }
    }

    async fn on_call_ended(&self, call: CallSession, reason: &str) {
        tracing::info!("Subscription test call {} ended: {}", call.id(), reason);
    }
}

/// Create a test session manager for subscription testing
async fn create_subscription_test_manager(behavior: HandlerBehavior) -> Result<Arc<SessionManager>, SessionError> {
    let handler = Arc::new(SubscriptionTestHandler::new(behavior));
    
    SessionManagerBuilder::new()
        .with_sip_bind_address("127.0.0.1")
        .with_sip_port(0)
        .with_from_uri("sip:subscription@localhost")
        .with_handler(handler)
        .build()
        .await
}

#[tokio::test]
async fn test_session_manager_event_handling() {
    let manager = create_subscription_test_manager(HandlerBehavior::AcceptAll).await.unwrap();
    
    manager.start().await.unwrap();
    
    // Test basic session creation to verify event handling is working
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("Test SDP".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Test that session events are handled properly
    let hold_result = manager.hold_session(&session_id).await;
    assert!(hold_result.is_ok());
    
    let resume_result = manager.resume_session(&session_id).await;
    assert!(resume_result.is_ok());
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_session_manager_with_accepting_behavior() {
    let manager = create_subscription_test_manager(HandlerBehavior::AcceptAll).await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create multiple sessions to test accepting behavior
    let mut sessions = Vec::new();
    
    for i in 0..3 {
        let call = manager.create_outgoing_call(
            &format!("sip:user{}@example.com", i),
            &format!("sip:target{}@example.com", i),
            Some(format!("SDP for session {}", i))
        ).await.unwrap();
        sessions.push(call.id().clone());
    }
    
    // Verify all sessions were created successfully
    let stats = manager.get_stats().await.unwrap();
    assert_eq!(stats.active_sessions, 3);
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_session_manager_with_rejecting_behavior() {
    let manager = create_subscription_test_manager(HandlerBehavior::RejectAll).await.unwrap();
    
    manager.start().await.unwrap();
    
    // Even with rejecting behavior, outgoing calls should work
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("Test SDP".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Test session operations
    let hold_result = manager.hold_session(&session_id).await;
    assert!(hold_result.is_ok());
    
    let resume_result = manager.resume_session(&session_id).await;
    assert!(resume_result.is_ok());
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_session_manager_with_selective_behavior() {
    let manager = create_subscription_test_manager(HandlerBehavior::AcceptSelective).await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create calls that should be accepted
    let accepted_call = manager.create_outgoing_call(
        "sip:accepted_user@example.com",
        "sip:target@example.com",
        Some("Accepted SDP".to_string())
    ).await.unwrap();
    
    // Create calls that would be rejected (if they were incoming)
    let rejected_call = manager.create_outgoing_call(
        "sip:rejected_user@example.com",
        "sip:target@example.com",
        Some("Rejected SDP".to_string())
    ).await.unwrap();
    
    // Both outgoing calls should succeed regardless of handler behavior
    assert!(accepted_call.id() != rejected_call.id());
    
    let stats = manager.get_stats().await.unwrap();
    assert_eq!(stats.active_sessions, 2);
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_session_lifecycle_events() {
    let manager = create_subscription_test_manager(HandlerBehavior::AcceptAll).await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create a session
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("Lifecycle test SDP".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Test various lifecycle events
    let hold_result = manager.hold_session(&session_id).await;
    assert!(hold_result.is_ok(), "Hold operation should succeed");
    
    let resume_result = manager.resume_session(&session_id).await;
    assert!(resume_result.is_ok(), "Resume operation should succeed");
    
    let dtmf_result = manager.send_dtmf(&session_id, "123").await;
    assert!(dtmf_result.is_ok(), "DTMF operation should succeed");
    
    let media_result = manager.update_media(&session_id, "Updated SDP").await;
    assert!(media_result.is_ok(), "Media update operation should succeed");
    
    let transfer_result = manager.transfer_session(&session_id, "sip:charlie@example.com").await;
    assert!(transfer_result.is_ok(), "Transfer operation should succeed");
    
    // Finally terminate
    let terminate_result = manager.terminate_session(&session_id).await;
    assert!(terminate_result.is_ok());
    
    // Wait for cleanup
    tokio::time::sleep(Duration::from_millis(50)).await;
    
    let final_stats = manager.get_stats().await.unwrap();
    assert_eq!(final_stats.active_sessions, 0);
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_concurrent_session_operations() {
    let manager = create_subscription_test_manager(HandlerBehavior::AcceptAll).await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create multiple sessions
    let mut sessions = Vec::new();
    for i in 0..5 {
        let call = manager.create_outgoing_call(
            &format!("sip:concurrent_user_{}@example.com", i),
            &format!("sip:target_{}@example.com", i),
            Some(format!("Concurrent SDP {}", i))
        ).await.unwrap();
        sessions.push(call.id().clone());
    }
    
    // Perform concurrent operations on all sessions
    let mut handles = Vec::new();
    
    for (i, session_id) in sessions.iter().enumerate() {
        let manager_clone: Arc<SessionManager> = Arc::clone(&manager);
        let session_id_clone = session_id.clone();
        
        let handle = tokio::spawn(async move {
            // Perform a sequence of operations
            let _ = manager_clone.hold_session(&session_id_clone).await;
            tokio::time::sleep(Duration::from_millis(1)).await;
            let _ = manager_clone.resume_session(&session_id_clone).await;
            tokio::time::sleep(Duration::from_millis(1)).await;
            let _ = manager_clone.send_dtmf(&session_id_clone, &format!("{}", i)).await;
        });
        handles.push(handle);
    }
    
    // Wait for all operations to complete
    for handle in handles {
        handle.await.unwrap();
    }
    
    // Verify all sessions are still active
    let stats = manager.get_stats().await.unwrap();
    assert_eq!(stats.active_sessions, 5);
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_session_state_consistency() {
    let manager = create_subscription_test_manager(HandlerBehavior::AcceptAll).await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create a session
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("State test SDP".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Check initial state
    let session = manager.find_session(&session_id).await.unwrap();
    assert!(session.is_some());
    
    // Perform state-changing operations
    manager.hold_session(&session_id).await.unwrap();
    
    // Session should still exist
    let session_after_hold = manager.find_session(&session_id).await.unwrap();
    assert!(session_after_hold.is_some());
    
    manager.resume_session(&session_id).await.unwrap();
    
    // Session should still exist
    let session_after_resume = manager.find_session(&session_id).await.unwrap();
    assert!(session_after_resume.is_some());
    
    // Terminate session
    manager.terminate_session(&session_id).await.unwrap();
    
    // Wait for cleanup
    tokio::time::sleep(Duration::from_millis(50)).await;
    
    // Session should be gone
    let session_after_terminate = manager.find_session(&session_id).await.unwrap();
    assert!(session_after_terminate.is_none());
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_rapid_session_creation_and_termination() {
    let manager = create_subscription_test_manager(HandlerBehavior::AcceptAll).await.unwrap();
    
    manager.start().await.unwrap();
    
    // Rapidly create and terminate sessions
    for i in 0..10 {
        let call = manager.create_outgoing_call(
            &format!("sip:rapid_user_{}@example.com", i),
            &format!("sip:target_{}@example.com", i),
            Some(format!("Rapid test SDP {}", i))
        ).await.unwrap();
        
        let session_id = call.id().clone();
        
        // Immediately terminate
        let terminate_result = manager.terminate_session(&session_id).await;
        assert!(terminate_result.is_ok());
        
        // Small delay
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    
    // Wait for cleanup
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    // Should have no active sessions
    let final_stats = manager.get_stats().await.unwrap();
    assert_eq!(final_stats.active_sessions, 0);
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_session_operation_ordering() {
    let manager = create_subscription_test_manager(HandlerBehavior::AcceptAll).await.unwrap();
    
    manager.start().await.unwrap();
    
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("Ordering test SDP".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Perform operations in a specific order and verify they all succeed
    let result = manager.hold_session(&session_id).await;
    assert!(result.is_ok(), "Operation 0 (hold) should succeed");
    
    let result = manager.send_dtmf(&session_id, "1").await;
    assert!(result.is_ok(), "Operation 1 (dtmf) should succeed");
    
    let result = manager.update_media(&session_id, "Updated SDP 1").await;
    assert!(result.is_ok(), "Operation 2 (media update) should succeed");
    
    let result = manager.resume_session(&session_id).await;
    assert!(result.is_ok(), "Operation 3 (resume) should succeed");
    
    let result = manager.send_dtmf(&session_id, "2").await;
    assert!(result.is_ok(), "Operation 4 (dtmf) should succeed");
    
    let result = manager.update_media(&session_id, "Updated SDP 2").await;
    assert!(result.is_ok(), "Operation 5 (media update) should succeed");
    
    let result = manager.transfer_session(&session_id, "sip:charlie@example.com").await;
    assert!(result.is_ok(), "Operation 6 (transfer) should succeed");
    
    let result = manager.send_dtmf(&session_id, "3").await;
    assert!(result.is_ok(), "Operation 7 (dtmf) should succeed");
    
    let result = manager.hold_session(&session_id).await;
    assert!(result.is_ok(), "Operation 8 (hold) should succeed");
    
    let result = manager.resume_session(&session_id).await;
    assert!(result.is_ok(), "Operation 9 (resume) should succeed");
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_error_recovery() {
    let manager = create_subscription_test_manager(HandlerBehavior::AcceptAll).await.unwrap();
    
    manager.start().await.unwrap();
    
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("Error recovery test SDP".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    let fake_session_id = SessionId::new();
    
    // Perform valid operation
    let valid_result = manager.hold_session(&session_id).await;
    assert!(valid_result.is_ok());
    
    // Perform invalid operation (should fail gracefully)
    let invalid_result = manager.hold_session(&fake_session_id).await;
    assert!(invalid_result.is_err());
    
    // Perform another valid operation (should still work)
    let recovery_result = manager.resume_session(&session_id).await;
    assert!(recovery_result.is_ok());
    
    manager.stop().await.unwrap();
} 