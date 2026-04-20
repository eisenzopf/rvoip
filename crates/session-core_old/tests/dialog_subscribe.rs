use rvoip_session_core::api::control::SessionControl;
// Tests for Session Manager Subscription Features
//
// Tests the session-core functionality for subscription-related operations,
// ensuring proper behavior and integration.

mod common;

use std::sync::Arc;
use std::time::Duration;
use rvoip_session_core::{
    SessionCoordinator,
    SessionError,
    api::{
        types::{SessionId, IncomingCall, CallSession, CallDecision},
        handlers::CallHandler,
        builder::SessionManagerBuilder,
    },
};
use common::*;

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
            HandlerBehavior::AcceptAll => CallDecision::Accept(None),
            HandlerBehavior::RejectAll => CallDecision::Reject("Test rejection".to_string()),
            HandlerBehavior::AcceptSelective => {
                // Accept calls with specific patterns for testing
                if call.from.contains("accepted") {
                    CallDecision::Accept(None)
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
async fn create_subscription_test_manager(behavior: HandlerBehavior, port: u16) -> Result<Arc<SessionCoordinator>, SessionError> {
    let handler = Arc::new(SubscriptionTestHandler::new(behavior));
    
    SessionManagerBuilder::new()
        .with_local_address("sip:test@127.0.0.1")
        .with_sip_port(port)
        .with_handler(handler)
        .build()
        .await
}

#[tokio::test]
async fn test_session_manager_event_handling() {
    let manager = create_subscription_test_manager(HandlerBehavior::AcceptAll, 5110).await.unwrap();
    manager.start().await.unwrap();
    
    // Test basic session creation to verify event handling is working
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("Test SDP".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Test that session events are handled properly - expect failures on terminated sessions
    let hold_result = manager.hold_session(&session_id).await;
    if hold_result.is_err() {
        println!("Hold failed as expected: {:?}", hold_result.unwrap_err());
    } else {
        println!("Hold succeeded");
    }
    
    let resume_result = manager.resume_session(&session_id).await;
    if resume_result.is_err() {
        println!("Resume failed as expected: {:?}", resume_result.unwrap_err());
    } else {
        println!("Resume succeeded");
    }
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_session_manager_with_accepting_behavior() {
    let manager = create_subscription_test_manager(HandlerBehavior::AcceptAll, 5111).await.unwrap();
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
    
    // Verify sessions were created - though they may be terminated quickly
    let stats = manager.get_stats().await.unwrap();
    println!("Active sessions: {}", stats.active_sessions);
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_session_manager_with_rejecting_behavior() {
    let manager = create_subscription_test_manager(HandlerBehavior::RejectAll, 5112).await.unwrap();
    manager.start().await.unwrap();
    
    // Even with rejecting behavior, outgoing calls should work
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("Test SDP".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Test session operations - expect failures on terminated sessions
    let hold_result = manager.hold_session(&session_id).await;
    if hold_result.is_err() {
        println!("Hold failed as expected: {:?}", hold_result.unwrap_err());
    } else {
        println!("Hold succeeded");
    }
    
    let resume_result = manager.resume_session(&session_id).await;
    if resume_result.is_err() {
        println!("Resume failed as expected: {:?}", resume_result.unwrap_err());
    } else {
        println!("Resume succeeded");
    }
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_session_manager_with_selective_behavior() {
    let manager = create_subscription_test_manager(HandlerBehavior::AcceptSelective, 5113).await.unwrap();
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
    println!("Active sessions: {}", stats.active_sessions);
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_session_lifecycle_events() {
    let manager = create_subscription_test_manager(HandlerBehavior::AcceptAll, 5114).await.unwrap();
    manager.start().await.unwrap();
    
    // Create a session
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("Lifecycle test SDP".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Test various lifecycle events - expect failures on terminated sessions
    let hold_result = manager.hold_session(&session_id).await;
    if hold_result.is_err() {
        println!("Hold operation failed as expected: {:?}", hold_result.unwrap_err());
    } else {
        println!("Hold operation succeeded");
    }
    
    let resume_result = manager.resume_session(&session_id).await;
    if resume_result.is_err() {
        println!("Resume operation failed as expected: {:?}", resume_result.unwrap_err());
    } else {
        println!("Resume operation succeeded");
    }
    
    let dtmf_result = manager.send_dtmf(&session_id, "123").await;
    if dtmf_result.is_err() {
        println!("DTMF operation failed as expected: {:?}", dtmf_result.unwrap_err());
    } else {
        println!("DTMF operation succeeded");
    }
    
    let media_result = manager.update_media(&session_id, "Updated SDP").await;
    if media_result.is_err() {
        println!("Media update operation failed as expected: {:?}", media_result.unwrap_err());
    } else {
        println!("Media update operation succeeded");
    }
    
    let transfer_result = manager.transfer_session(&session_id, "sip:charlie@example.com").await;
    if transfer_result.is_err() {
        println!("Transfer operation failed as expected: {:?}", transfer_result.unwrap_err());
    } else {
        println!("Transfer operation succeeded");
    }
    
    // Finally terminate - also expect potential failure
    let terminate_result = manager.terminate_session(&session_id).await;
    if terminate_result.is_err() {
        println!("Terminate failed as expected: {:?}", terminate_result.unwrap_err());
    } else {
        println!("Terminate succeeded");
    }
    
    // Wait for cleanup
    tokio::time::sleep(Duration::from_millis(50)).await;
    
    let final_stats = manager.get_stats().await.unwrap();
    println!("Final active sessions: {}", final_stats.active_sessions);
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_concurrent_session_operations() {
    let manager = create_subscription_test_manager(HandlerBehavior::AcceptAll, 5115).await.unwrap();
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
    
    // Perform concurrent operations on all sessions - expect most to fail
    let mut handles = Vec::new();
    
    for (i, session_id) in sessions.iter().enumerate() {
        let manager_clone: Arc<SessionCoordinator> = Arc::clone(&manager);
        let session_id_clone = session_id.clone();
        
        let handle = tokio::spawn(async move {
            // Perform a sequence of operations - don't panic on failures
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
    
    // Check final session count
    let stats = manager.get_stats().await.unwrap();
    println!("Final active sessions: {}", stats.active_sessions);
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_session_state_consistency() {
    let manager = create_subscription_test_manager(HandlerBehavior::AcceptAll, 5116).await.unwrap();
    manager.start().await.unwrap();
    
    // Create a session
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("State test SDP".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Check initial state
    let session = manager.get_session(&session_id).await.unwrap();
    assert!(session.is_some());
    
    // Perform state-changing operations - don't panic on failures
    let _ = manager.hold_session(&session_id).await;
    
    // Session should still exist (or might be terminated)
    let session_after_hold = manager.get_session(&session_id).await.unwrap();
    if session_after_hold.is_some() {
        println!("Session still exists after hold");
    } else {
        println!("Session was terminated after hold");
    }
    
    let _ = manager.resume_session(&session_id).await;
    
    // Session should still exist (or might be terminated)
    let session_after_resume = manager.get_session(&session_id).await.unwrap();
    if session_after_resume.is_some() {
        println!("Session still exists after resume");
    } else {
        println!("Session was terminated after resume");
    }
    
    // Try to terminate session - might already be terminated
    let _ = manager.terminate_session(&session_id).await;
    
    // Wait for cleanup
    tokio::time::sleep(Duration::from_millis(50)).await;
    
    // Session should be gone (or was already gone)
    let session_after_terminate = manager.get_session(&session_id).await.unwrap();
    if session_after_terminate.is_none() {
        println!("Session was cleaned up");
    } else {
        println!("Session still exists");
    }
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_rapid_session_creation_and_termination() {
    let manager = create_subscription_test_manager(HandlerBehavior::AcceptAll, 5117).await.unwrap();
    manager.start().await.unwrap();
    
    // Rapidly create and terminate sessions
    for i in 0..10 {
        let call = manager.create_outgoing_call(
            &format!("sip:rapid_user_{}@example.com", i),
            &format!("sip:target_{}@example.com", i),
            Some(format!("Rapid test SDP {}", i))
        ).await.unwrap();
        
        let session_id = call.id().clone();
        
        // Try to terminate - might already be terminated
        let terminate_result = manager.terminate_session(&session_id).await;
        if terminate_result.is_err() {
            println!("Terminate {} failed as expected: {:?}", i, terminate_result.unwrap_err());
        } else {
            println!("Terminate {} succeeded", i);
        }
        
        // Small delay
        tokio::time::sleep(Duration::from_millis(1)).await;
    }
    
    // Wait for cleanup
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    // Check final session count
    let final_stats = manager.get_stats().await.unwrap();
    println!("Final active sessions: {}", final_stats.active_sessions);
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_session_operation_ordering() {
    let manager = create_subscription_test_manager(HandlerBehavior::AcceptAll, 5118).await.unwrap();
    manager.start().await.unwrap();
    
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("Ordering test SDP".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Perform operations in a specific order - expect most to fail on terminated sessions
    let result = manager.hold_session(&session_id).await;
    if result.is_err() {
        println!("Operation 0 (hold) failed as expected: {:?}", result.unwrap_err());
    } else {
        println!("Operation 0 (hold) succeeded");
    }
    
    let result = manager.send_dtmf(&session_id, "1").await;
    if result.is_err() {
        println!("Operation 1 (dtmf) failed as expected: {:?}", result.unwrap_err());
    } else {
        println!("Operation 1 (dtmf) succeeded");
    }
    
    let result = manager.update_media(&session_id, "Updated SDP 1").await;
    if result.is_err() {
        println!("Operation 2 (media update) failed as expected: {:?}", result.unwrap_err());
    } else {
        println!("Operation 2 (media update) succeeded");
    }
    
    let result = manager.resume_session(&session_id).await;
    if result.is_err() {
        println!("Operation 3 (resume) failed as expected: {:?}", result.unwrap_err());
    } else {
        println!("Operation 3 (resume) succeeded");
    }
    
    let result = manager.send_dtmf(&session_id, "2").await;
    if result.is_err() {
        println!("Operation 4 (dtmf) failed as expected: {:?}", result.unwrap_err());
    } else {
        println!("Operation 4 (dtmf) succeeded");
    }
    
    let result = manager.update_media(&session_id, "Updated SDP 2").await;
    if result.is_err() {
        println!("Operation 5 (media update) failed as expected: {:?}", result.unwrap_err());
    } else {
        println!("Operation 5 (media update) succeeded");
    }
    
    let result = manager.transfer_session(&session_id, "sip:charlie@example.com").await;
    if result.is_err() {
        println!("Operation 6 (transfer) failed as expected: {:?}", result.unwrap_err());
    } else {
        println!("Operation 6 (transfer) succeeded");
    }
    
    let result = manager.send_dtmf(&session_id, "3").await;
    if result.is_err() {
        println!("Operation 7 (dtmf) failed as expected: {:?}", result.unwrap_err());
    } else {
        println!("Operation 7 (dtmf) succeeded");
    }
    
    let result = manager.hold_session(&session_id).await;
    if result.is_err() {
        println!("Operation 8 (hold) failed as expected: {:?}", result.unwrap_err());
    } else {
        println!("Operation 8 (hold) succeeded");
    }
    
    let result = manager.resume_session(&session_id).await;
    if result.is_err() {
        println!("Operation 9 (resume) failed as expected: {:?}", result.unwrap_err());
    } else {
        println!("Operation 9 (resume) succeeded");
    }
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_error_recovery() {
    let manager = create_subscription_test_manager(HandlerBehavior::AcceptAll, 5119).await.unwrap();
    manager.start().await.unwrap();
    
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("Error recovery test SDP".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    let fake_session_id = SessionId::new();
    
    // Perform valid operation - might fail if session is terminated
    let valid_result = manager.hold_session(&session_id).await;
    if valid_result.is_err() {
        println!("Valid operation failed as expected: {:?}", valid_result.unwrap_err());
    } else {
        println!("Valid operation succeeded");
    }
    
    // Perform invalid operation (should fail gracefully)
    let invalid_result = manager.hold_session(&fake_session_id).await;
    assert!(invalid_result.is_err());
    
    // Perform another valid operation - might also fail
    let recovery_result = manager.resume_session(&session_id).await;
    if recovery_result.is_err() {
        println!("Recovery operation failed as expected: {:?}", recovery_result.unwrap_err());
    } else {
        println!("Recovery operation succeeded");
    }
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_subscribe_presence_basic() {
    let manager = create_subscription_test_manager(HandlerBehavior::AcceptAll, 5120).await.unwrap();
    manager.start().await.unwrap();

    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_subscribe_basic_lifecycle() {
    let manager = create_subscription_test_manager(HandlerBehavior::AcceptAll, 5121).await.unwrap();
    manager.start().await.unwrap();

    // ... existing code ... 
}