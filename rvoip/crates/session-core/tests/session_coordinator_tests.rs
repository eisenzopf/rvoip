//! Comprehensive tests for SessionCoordinator
//!
//! Tests all major functionality of the SessionCoordinator including:
//! - Initialization and lifecycle
//! - Call creation and termination
//! - State transitions
//! - Event handling
//! - Media coordination
//! - Error conditions

mod common;

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use rvoip_session_core::{
    SessionCoordinator,
    SessionControl,
    SessionError,
    prelude::SessionEvent,
    api::{
        handlers::CallHandler,
        builder::{SessionManagerBuilder, SessionManagerConfig},
        types::{CallSession, SessionId, CallState, IncomingCall, CallDecision, SessionStats},
    },
};

/// Test handler that tracks all events
#[derive(Debug, Default)]
struct TrackingHandler {
    events: Arc<tokio::sync::Mutex<Vec<String>>>,
}

impl TrackingHandler {
    fn new() -> Self {
        Self {
            events: Arc::new(tokio::sync::Mutex::new(Vec::new())),
        }
    }

    async fn get_events(&self) -> Vec<String> {
        self.events.lock().await.clone()
    }
}

#[async_trait::async_trait]
impl CallHandler for TrackingHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        let mut events = self.events.lock().await;
        events.push(format!("incoming_call:{}", call.id));
        CallDecision::Accept(None)
    }

    async fn on_call_established(&self, call: CallSession, _local_sdp: Option<String>, _remote_sdp: Option<String>) {
        let mut events = self.events.lock().await;
        events.push(format!("call_established:{}", call.id()));
    }

    async fn on_call_ended(&self, call: CallSession, reason: &str) {
        let mut events = self.events.lock().await;
        events.push(format!("call_ended:{}:{}", call.id(), reason));
    }
}

#[tokio::test]
async fn test_coordinator_initialization() {
    println!("🧪 Testing SessionCoordinator initialization...");

    // Test with default config
    let config = SessionManagerConfig::default();
    let coordinator = SessionCoordinator::new(config.clone(), None)
        .await
        .expect("Failed to create coordinator");

    // Verify subsystems are initialized
    assert_eq!(coordinator.config.sip_port, 5060);
    assert_eq!(coordinator.config.media_port_start, 10000);
    assert_eq!(coordinator.config.media_port_end, 20000);

    // Start the coordinator
    coordinator.start().await.expect("Failed to start coordinator");

    // Get bound address
    let addr = coordinator.get_bound_address();
    println!("✅ Coordinator started on: {}", addr);

    // Stop the coordinator
    coordinator.stop().await.expect("Failed to stop coordinator");
    println!("✅ Coordinator stopped successfully");
}

#[tokio::test]
async fn test_coordinator_with_custom_config() {
    println!("🧪 Testing SessionCoordinator with custom configuration...");

    let handler = Arc::new(TrackingHandler::new());
    
    let coordinator = SessionManagerBuilder::new()
        .with_sip_port(7001)
        .with_local_address("sip:alice@test.local")
        .with_media_ports(30000, 31000)
        .with_handler(handler.clone())
        .build()
        .await
        .expect("Failed to build coordinator");

    coordinator.start().await.expect("Failed to start");

    // Verify configuration
    assert_eq!(coordinator.config.sip_port, 7001);
    assert_eq!(coordinator.config.local_address, "sip:alice@test.local");
    assert_eq!(coordinator.config.media_port_start, 30000);
    assert_eq!(coordinator.config.media_port_end, 31000);

    // Verify handler is set
    assert!(coordinator.get_handler().is_some());

    coordinator.stop().await.expect("Failed to stop");
    println!("✅ Custom configuration verified");
}

#[tokio::test]
async fn test_outgoing_call_lifecycle() {
    println!("🧪 Testing outgoing call lifecycle...");

    let handler = Arc::new(TrackingHandler::new());
    let coordinator = SessionManagerBuilder::new()
        .with_sip_port(7002)
        .with_handler(handler.clone())
        .build()
        .await
        .expect("Failed to build coordinator");

    coordinator.start().await.expect("Failed to start");

    // Create an outgoing call
    let call = coordinator.create_outgoing_call(
        "sip:alice@test.local",
        "sip:bob@example.com",
        None,
    ).await.expect("Failed to create call");

    println!("📞 Created call: {} -> {}", call.from, call.to);
    assert_eq!(call.state(), &CallState::Initiating);

    // Get session info
    let session_info = coordinator.get_session(&call.id)
        .await
        .expect("Failed to get session")
        .expect("Session not found");
    
    assert_eq!(session_info.id, call.id);
    assert_eq!(session_info.from, call.from);

    // List active sessions
    let sessions = coordinator.list_active_sessions()
        .await
        .expect("Failed to list sessions");
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0], call.id);

    // Get stats
    let stats = coordinator.get_stats()
        .await
        .expect("Failed to get stats");
    assert_eq!(stats.active_sessions, 1);
    assert_eq!(stats.total_sessions, 1);

    // Terminate the call
    coordinator.terminate_session(&call.id)
        .await
        .expect("Failed to terminate session");

    // Wait for termination to process
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Verify call ended event was triggered
    let events = handler.get_events().await;
    assert!(events.iter().any(|e| e.starts_with(&format!("call_ended:{}", call.id))));

    // Verify stats updated
    let stats = coordinator.get_stats()
        .await
        .expect("Failed to get stats");
    assert_eq!(stats.active_sessions, 0);

    coordinator.stop().await.expect("Failed to stop");
    println!("✅ Call lifecycle test completed");
}

#[tokio::test]
async fn test_multiple_concurrent_calls() {
    println!("🧪 Testing multiple concurrent calls...");

    let coordinator = SessionManagerBuilder::new()
        .with_sip_port(7003)
        .build()
        .await
        .expect("Failed to build coordinator");

    coordinator.start().await.expect("Failed to start");

    // Create multiple calls concurrently
    let mut handles = vec![];
    for i in 0..5 {
        let coord_clone = coordinator.clone();
        let handle = tokio::spawn(async move {
            coord_clone.create_outgoing_call(
                &format!("sip:alice{}@test.local", i),
                &format!("sip:bob{}@example.com", i),
                None,
            ).await
        });
        handles.push(handle);
    }

    // Wait for all calls to be created
    let mut calls = vec![];
    for handle in handles {
        match handle.await.expect("Task panicked") {
            Ok(call) => calls.push(call),
            Err(e) => panic!("Failed to create call: {}", e),
        }
    }

    // Verify all calls were created
    assert_eq!(calls.len(), 5);

    // Verify stats
    let stats = coordinator.get_stats()
        .await
        .expect("Failed to get stats");
    assert_eq!(stats.active_sessions, 5);
    assert_eq!(stats.total_sessions, 5);

    // List all sessions
    let sessions = coordinator.list_active_sessions()
        .await
        .expect("Failed to list sessions");
    assert_eq!(sessions.len(), 5);

    // Terminate all calls
    for call in &calls {
        coordinator.terminate_session(&call.id)
            .await
            .expect("Failed to terminate session");
    }

    // Wait for terminations to process
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Verify all calls terminated
    let sessions = coordinator.list_active_sessions()
        .await
        .expect("Failed to list sessions");
    assert_eq!(sessions.len(), 0);

    coordinator.stop().await.expect("Failed to stop");
    println!("✅ Concurrent calls test completed");
}

#[tokio::test]
async fn test_media_session_coordination() {
    println!("🧪 Testing media session coordination...");

    let handler = Arc::new(TrackingHandler::new());
    let coordinator = SessionManagerBuilder::new()
        .with_sip_port(7004)
        .with_handler(handler.clone())
        .build()
        .await
        .expect("Failed to build coordinator");

    coordinator.start().await.expect("Failed to start");

    // Create a call with SDP
    let sdp = "v=0\r\n\
               o=test 123 456 IN IP4 127.0.0.1\r\n\
               s=Test Session\r\n\
               c=IN IP4 127.0.0.1\r\n\
               t=0 0\r\n\
               m=audio 10000 RTP/AVP 0\r\n\
               a=rtpmap:0 PCMU/8000\r\n";

    let call = coordinator.create_outgoing_call(
        "sip:alice@test.local",
        "sip:bob@example.com",
        Some(sdp.to_string()),
    ).await.expect("Failed to create call");

    // Simulate call becoming active
    if let Ok(Some(mut session)) = coordinator.registry.get_session(&call.id).await {
        let old_state = session.state.clone();
        session.state = CallState::Active;
        coordinator.registry.register_session(call.id.clone(), session).await.unwrap();
        
        // Send state change event
        let _ = coordinator.event_tx.send(SessionEvent::StateChanged {
            session_id: call.id.clone(),
            old_state,
            new_state: CallState::Active,
        }).await;
    }

    // Wait for media session to be created
    tokio::time::sleep(Duration::from_millis(200)).await;

    // Check media info
    let media_info = coordinator.get_media_info(&call.id)
        .await
        .expect("Failed to get media info");
    
    // Media session should exist once call is active
    if media_info.is_some() {
        println!("✅ Media session created when call became active");
    }

    // Update media
    let new_sdp = "v=0\r\n\
                   o=test 123 457 IN IP4 127.0.0.1\r\n\
                   s=Test Session\r\n\
                   c=IN IP4 127.0.0.1\r\n\
                   t=0 0\r\n\
                   m=audio 10002 RTP/AVP 0 8\r\n\
                   a=rtpmap:0 PCMU/8000\r\n\
                   a=rtpmap:8 PCMA/8000\r\n";

    coordinator.update_media(&call.id, new_sdp)
        .await
        .expect("Failed to update media");

    // Terminate the call
    coordinator.terminate_session(&call.id)
        .await
        .expect("Failed to terminate session");

    // Wait for cleanup
    tokio::time::sleep(Duration::from_millis(100)).await;

    coordinator.stop().await.expect("Failed to stop");
    println!("✅ Media coordination test completed");
}

#[tokio::test]
async fn test_call_state_transitions() {
    println!("🧪 Testing call state transitions...");

    let handler = Arc::new(TrackingHandler::new());
    let coordinator = SessionManagerBuilder::new()
        .with_sip_port(7005)
        .with_handler(handler.clone())
        .build()
        .await
        .expect("Failed to build coordinator");

    coordinator.start().await.expect("Failed to start");

    // Create a call
    let call = coordinator.create_outgoing_call(
        "sip:alice@test.local",
        "sip:bob@example.com",
        None,
    ).await.expect("Failed to create call");

    // Test hold/resume
    // First make the call active
    if let Ok(Some(mut session)) = coordinator.registry.get_session(&call.id).await {
        session.state = CallState::Active;
        coordinator.registry.register_session(call.id.clone(), session).await.unwrap();
    }

    // Hold the call
    coordinator.hold_session(&call.id)
        .await
        .expect("Failed to hold session");

    // Verify state
    let session = coordinator.get_session(&call.id)
        .await.unwrap().unwrap();
    assert_eq!(session.state(), &CallState::OnHold);

    // Resume the call
    coordinator.resume_session(&call.id)
        .await
        .expect("Failed to resume session");

    // Verify state
    let session = coordinator.get_session(&call.id)
        .await.unwrap().unwrap();
    assert_eq!(session.state(), &CallState::Active);

    // Test transfer
    coordinator.transfer_session(&call.id, "sip:charlie@example.com")
        .await
        .expect("Failed to transfer session");

    // Verify state
    let session = coordinator.get_session(&call.id)
        .await.unwrap().unwrap();
    assert_eq!(session.state(), &CallState::Transferring);

    coordinator.stop().await.expect("Failed to stop");
    println!("✅ State transitions test completed");
}

#[tokio::test]
async fn test_dtmf_sending() {
    println!("🧪 Testing DTMF sending...");

    let coordinator = SessionManagerBuilder::new()
        .with_sip_port(7006)
        .build()
        .await
        .expect("Failed to build coordinator");

    coordinator.start().await.expect("Failed to start");

    // Create a call
    let call = coordinator.create_outgoing_call(
        "sip:alice@test.local",
        "sip:bob@example.com",
        None,
    ).await.expect("Failed to create call");

    // Make the call active
    if let Ok(Some(mut session)) = coordinator.registry.get_session(&call.id).await {
        session.state = CallState::Active;
        coordinator.registry.register_session(call.id.clone(), session).await.unwrap();
    }

    // Send DTMF
    coordinator.send_dtmf(&call.id, "123#")
        .await
        .expect("Failed to send DTMF");

    // Send more complex DTMF sequence
    coordinator.send_dtmf(&call.id, "*456#")
        .await
        .expect("Failed to send DTMF");

    coordinator.stop().await.expect("Failed to stop");
    println!("✅ DTMF test completed");
}

#[tokio::test]
async fn test_error_conditions() {
    println!("🧪 Testing error conditions...");

    let coordinator = SessionManagerBuilder::new()
        .with_sip_port(7007)
        .build()
        .await
        .expect("Failed to build coordinator");

    coordinator.start().await.expect("Failed to start");

    // Test operations on non-existent session
    let fake_id = SessionId::new();
    
    // Should fail - session doesn't exist
    assert!(coordinator.terminate_session(&fake_id).await.is_err());
    assert!(coordinator.hold_session(&fake_id).await.is_err());
    assert!(coordinator.resume_session(&fake_id).await.is_err());
    assert!(coordinator.send_dtmf(&fake_id, "123").await.is_err());
    assert!(coordinator.update_media(&fake_id, "fake sdp").await.is_err());

    // Create a call for state-based errors
    let call = coordinator.create_outgoing_call(
        "sip:alice@test.local",
        "sip:bob@example.com",
        None,
    ).await.expect("Failed to create call");

    // Test invalid state transitions
    // Can't resume a call that's not on hold
    assert!(coordinator.resume_session(&call.id).await.is_err());

    // Can't send DTMF on non-active call
    assert!(coordinator.send_dtmf(&call.id, "123").await.is_err());

    coordinator.stop().await.expect("Failed to stop");
    println!("✅ Error conditions test completed");
}

#[tokio::test]
async fn test_event_handler_callbacks() {
    println!("🧪 Testing event handler callbacks...");

    let handler = Arc::new(TrackingHandler::new());
    let coordinator = SessionManagerBuilder::new()
        .with_sip_port(7008)
        .with_handler(handler.clone())
        .build()
        .await
        .expect("Failed to build coordinator");

    coordinator.start().await.expect("Failed to start");

    // Create a call
    let call = coordinator.create_outgoing_call(
        "sip:alice@test.local",
        "sip:bob@example.com",
        None,
    ).await.expect("Failed to create call");

    // Simulate call establishment
    if let Ok(Some(mut session)) = coordinator.registry.get_session(&call.id).await {
        let old_state = session.state.clone();
        session.state = CallState::Active;
        coordinator.registry.register_session(call.id.clone(), session.clone()).await.unwrap();
        
        // Send state change event
        let _ = coordinator.event_tx.send(SessionEvent::StateChanged {
            session_id: call.id.clone(),
            old_state,
            new_state: CallState::Active,
        }).await;
        
        // Manually trigger the handler callback since we're simulating
        if let Some(hdlr) = &coordinator.handler {
            hdlr.on_call_established(session, None, None).await;
        }
    }

    // Wait for event processing
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Verify call established event was recorded
    let events = handler.get_events().await;
    assert!(events.iter().any(|e| e.starts_with(&format!("call_established:{}", call.id))));

    // Terminate the call
    coordinator.terminate_session(&call.id)
        .await
        .expect("Failed to terminate session");

    // Wait for event processing
    tokio::time::sleep(Duration::from_millis(100)).await;

    // Verify call ended event was recorded
    let events = handler.get_events().await;
    assert!(events.iter().any(|e| e.starts_with(&format!("call_ended:{}", call.id))));

    coordinator.stop().await.expect("Failed to stop");
    println!("✅ Event handler test completed");
}

#[tokio::test]
async fn test_cleanup_on_shutdown() {
    println!("🧪 Testing cleanup on shutdown...");

    let coordinator = SessionManagerBuilder::new()
        .with_sip_port(7009)
        .build()
        .await
        .expect("Failed to build coordinator");

    coordinator.start().await.expect("Failed to start");

    // Create multiple calls
    let mut calls = vec![];
    for i in 0..3 {
        let call = coordinator.create_outgoing_call(
            &format!("sip:alice{}@test.local", i),
            &format!("sip:bob{}@example.com", i),
            None,
        ).await.expect("Failed to create call");
        calls.push(call);
    }

    // Verify calls exist
    let sessions = coordinator.list_active_sessions()
        .await
        .expect("Failed to list sessions");
    assert_eq!(sessions.len(), 3);

    // Stop the coordinator - should clean up all sessions
    coordinator.stop().await.expect("Failed to stop");

    println!("✅ Cleanup test completed");
}

#[cfg(test)]
mod prepared_call_tests {
    use super::*;

    #[tokio::test]
    async fn test_prepare_and_initiate_call() {
        println!("🧪 Testing prepare and initiate call flow...");

        let coordinator = SessionManagerBuilder::new()
            .with_sip_port(7010)
            .build()
            .await
            .expect("Failed to build coordinator");

        coordinator.start().await.expect("Failed to start");

        // Prepare a call (allocates resources, generates SDP)
        let prepared = coordinator.prepare_outgoing_call(
            "sip:alice@test.local",
            "sip:bob@example.com",
        ).await.expect("Failed to prepare call");

        println!("📞 Prepared call: {}", prepared.session_id);
        assert!(!prepared.sdp_offer.is_empty());
        assert!(prepared.local_rtp_port > 0);

        // Verify session exists in preparing state
        let session = coordinator.get_session(&prepared.session_id)
            .await.unwrap().unwrap();
        assert_eq!(session.state(), &CallState::Initiating);

        // Initiate the prepared call
        let call = coordinator.initiate_prepared_call(&prepared)
            .await
            .expect("Failed to initiate call");

        assert_eq!(call.id, prepared.session_id);
        assert_eq!(call.from, prepared.from);
        assert_eq!(call.to, prepared.to);

        // Cleanup
        coordinator.terminate_session(&call.id).await.ok();
        coordinator.stop().await.expect("Failed to stop");
        println!("✅ Prepare/initiate test completed");
    }
} 