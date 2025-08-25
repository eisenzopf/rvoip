// Tests for CANCEL Dialog Integration
//
// Tests the session-core functionality for CANCEL requests,
// ensuring proper integration with the underlying dialog layer.
// These tests use delayed response handling to test actual CANCEL
// functionality (early dialog termination before call establishment).

mod common;

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use rvoip_session_core::{
    SessionCoordinator,
    SessionError,
    api::{
        types::{CallState, SessionId, IncomingCall, CallSession, CallDecision},
        handlers::CallHandler,
        builder::SessionManagerBuilder,
    },
    manager::events::SessionEvent,
};
use common::*;

/// Handler that delays accepting calls to allow CANCEL testing
#[derive(Debug)]
struct DelayedAcceptHandler {
    delay: Duration,
    auto_accept: bool,
}

impl DelayedAcceptHandler {
    fn new(delay: Duration, auto_accept: bool) -> Self {
        Self { delay, auto_accept }
    }
}

#[async_trait::async_trait]
impl CallHandler for DelayedAcceptHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        // Delay before making decision to allow CANCEL
        tokio::time::sleep(self.delay).await;
        
        if self.auto_accept {
            CallDecision::Accept(Some("SDP answer".to_string()))
        } else {
            CallDecision::Defer
        }
    }

    async fn on_call_ended(&self, call: CallSession, reason: &str) {
        tracing::info!("Call {} ended with reason: {}", call.id(), reason);
    }
}



#[tokio::test]
async fn test_cancel_before_response() {
    // Create managers - B defers response to allow CANCEL
    let (manager_a, _, _) = create_session_manager_pair().await.unwrap();
    
    // Create handler that defers (never accepts)
    let handler = Arc::new(DelayedAcceptHandler::new(Duration::from_secs(5), false));
    let manager_b = SessionManagerBuilder::new()
        .with_local_address("sip:test@127.0.0.1")
        .with_sip_port(6100)
        .with_handler(handler)
        .build()
        .await
        .unwrap();
    
    // Subscribe to events before creating call
    let mut event_sub = manager_a.event_processor.subscribe().await.unwrap();
    
    // Create outgoing call
    let call = manager_a.create_outgoing_call(
        "sip:alice@127.0.0.1",
        &format!("sip:bob@{}", manager_b.get_bound_address()),
        Some("SDP offer".to_string()),
        None
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Verify call is in Initiating state
    verify_session_exists(&manager_a, &session_id, Some(&CallState::Initiating)).await.unwrap();
    
    // Cancel before any final response (100 Trying should have been received)
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    // This should send CANCEL since dialog is not established
    let terminate_result = manager_a.terminate_session(&session_id).await;
    println!("CANCEL result: {:?}", terminate_result);
    assert!(terminate_result.is_ok());
    
    // Wait for state transition to Terminated
    let terminated = wait_for_terminated_state(&mut event_sub, &session_id, Duration::from_secs(2)).await;
    assert!(terminated, "Session should transition to Terminated state");
    
    // Session might still exist but should be in Terminated state
    tokio::time::sleep(Duration::from_millis(100)).await;
}

#[tokio::test]
async fn test_cancel_after_provisional_response() {
    // Create managers with handler that sends provisional response
    let (manager_a, _, _) = create_session_manager_pair().await.unwrap();
    
    // Create custom handler that defers (simulates provisional response state)
    #[derive(Debug)]
    struct RingingHandler;
    #[async_trait::async_trait]
    impl CallHandler for RingingHandler {
        async fn on_incoming_call(&self, _call: IncomingCall) -> CallDecision {
            // Defer to simulate early dialog with provisional response
            // In a real scenario, this would send 180 Ringing
            CallDecision::Defer
        }
        
        async fn on_call_ended(&self, _call: CallSession, _reason: &str) {}
    }
    
    let manager_b = SessionManagerBuilder::new()
        .with_local_address("sip:test@127.0.0.1")
        .with_sip_port(6101)
        .with_handler(Arc::new(RingingHandler))
        .build()
        .await
        .unwrap();
    
    // Subscribe to events
    let mut event_sub = manager_a.event_processor.subscribe().await.unwrap();
    
    // Create outgoing call
    let call = manager_a.create_outgoing_call(
        "sip:alice@127.0.0.1",
        &format!("sip:bob@{}", manager_b.get_bound_address()),
        Some("SDP offer".to_string()),
        None
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Wait for initial processing (100 Trying received)
    tokio::time::sleep(Duration::from_millis(200)).await;
    
    // Cancel during early dialog
    let terminate_result = manager_a.terminate_session(&session_id).await;
    println!("CANCEL after provisional result: {:?}", terminate_result);
    assert!(terminate_result.is_ok());
    
    // Wait for state transition to Terminated
    let terminated = wait_for_terminated_state(&mut event_sub, &session_id, Duration::from_secs(2)).await;
    assert!(terminated, "Session should transition to Terminated state");
    
    // Session might still exist but should be in Terminated state
    tokio::time::sleep(Duration::from_millis(100)).await;
} 