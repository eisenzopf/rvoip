//! Tests for Session Manager Configuration and Capabilities
//!
//! Tests the session-core functionality for different configurations,
//! ensuring proper behavior across various scenarios.

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

/// Handler that tracks configuration options
#[derive(Debug)]
struct CapabilitiesTestHandler {
    accept_calls: bool,
}

impl CapabilitiesTestHandler {
    fn new(accept_calls: bool) -> Self {
        Self { accept_calls }
    }
}

#[async_trait::async_trait]
impl CallHandler for CapabilitiesTestHandler {
    async fn on_incoming_call(&self, _call: IncomingCall) -> CallDecision {
        if self.accept_calls {
            CallDecision::Accept
        } else {
            CallDecision::Reject("Capabilities test - not accepting calls".to_string())
        }
    }

    async fn on_call_ended(&self, call: CallSession, reason: &str) {
        tracing::info!("Capabilities test call {} ended: {}", call.id(), reason);
    }
}

/// Create a test session manager with specific capabilities
async fn create_capabilities_test_manager(accept_calls: bool) -> Result<Arc<SessionManager>, SessionError> {
    let handler = Arc::new(CapabilitiesTestHandler::new(accept_calls));
    
    SessionManagerBuilder::new()
        .with_sip_bind_address("127.0.0.1")
        .with_sip_port(0)
        .with_from_uri("sip:capabilities@localhost")
        .with_handler(handler)
        .build()
        .await
}

#[tokio::test]
async fn test_session_manager_basic_configuration() {
    let manager = create_capabilities_test_manager(true).await.unwrap();
    
    manager.start().await.unwrap();
    
    // Test basic functionality
    let stats = manager.get_stats().await.unwrap();
    assert_eq!(stats.active_sessions, 0);
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_session_manager_with_accepting_handler() {
    let manager = create_capabilities_test_manager(true).await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create outgoing call to test capabilities
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("Test SDP".to_string())
    ).await.unwrap();
    
    // Test session operations
    let session_id = call.id().clone();
    
    // Test hold capability
    let hold_result = manager.hold_session(&session_id).await;
    assert!(hold_result.is_ok());
    
    // Test resume capability
    let resume_result = manager.resume_session(&session_id).await;
    assert!(resume_result.is_ok());
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_session_manager_with_rejecting_handler() {
    let manager = create_capabilities_test_manager(false).await.unwrap();
    
    manager.start().await.unwrap();
    
    // Even with rejecting handler, outgoing calls should work
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("Test SDP".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Test that capabilities still work
    let transfer_result = manager.transfer_session(&session_id, "sip:charlie@example.com").await;
    assert!(transfer_result.is_ok());
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_dtmf_capabilities() {
    let manager = create_capabilities_test_manager(true).await.unwrap();
    
    manager.start().await.unwrap();
    
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("Test SDP".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Test DTMF capabilities with various inputs
    let dtmf_tests = vec![
        "0", "1", "2", "3", "4", "5", "6", "7", "8", "9",
        "*", "#", "A", "B", "C", "D",
        "123", "*0#", "123456789*0#ABCD"
    ];
    
    for dtmf in dtmf_tests {
        let result = manager.send_dtmf(&session_id, dtmf).await;
        assert!(result.is_ok(), "DTMF '{}' should be supported", dtmf);
    }
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_media_update_capabilities() {
    let manager = create_capabilities_test_manager(true).await.unwrap();
    
    manager.start().await.unwrap();
    
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("Initial SDP".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Test various media update scenarios
    let media_updates = vec![
        "v=0\r\no=alice 123 456 IN IP4 192.168.1.100\r\nc=IN IP4 192.168.1.100\r\nm=audio 5004 RTP/AVP 0\r\n",
        "v=0\r\no=alice 123 457 IN IP4 192.168.1.100\r\nc=IN IP4 192.168.1.100\r\nm=video 5006 RTP/AVP 96\r\n",
        "updated SDP with codec changes",
        "",  // Empty SDP
    ];
    
    for (i, sdp) in media_updates.iter().enumerate() {
        let result = manager.update_media(&session_id, sdp).await;
        assert!(result.is_ok(), "Media update {} should be supported", i);
    }
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_transfer_capabilities() {
    let manager = create_capabilities_test_manager(true).await.unwrap();
    
    manager.start().await.unwrap();
    
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("Test SDP".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Test various transfer targets
    let transfer_targets = vec![
        "sip:charlie@example.com",
        "sip:david@another-domain.com",
        "sip:1234@pbx.company.com",
        "sip:conference@meetings.example.com",
    ];
    
    for target in transfer_targets {
        let result = manager.transfer_session(&session_id, target).await;
        assert!(result.is_ok(), "Transfer to '{}' should be supported", target);
    }
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_concurrent_session_capabilities() {
    let manager = create_capabilities_test_manager(true).await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create multiple sessions to test concurrent capabilities
    let mut sessions = Vec::new();
    
    for i in 0..5 {
        let call = manager.create_outgoing_call(
            &format!("sip:caller{}@example.com", i),
            &format!("sip:target{}@example.com", i),
            Some(format!("SDP for session {}", i))
        ).await.unwrap();
        sessions.push(call.id().clone());
    }
    
    // Test that all sessions support the same capabilities
    for (i, session_id) in sessions.iter().enumerate() {
        let hold_result = manager.hold_session(session_id).await;
        assert!(hold_result.is_ok(), "Session {} should support hold", i);
        
        let resume_result = manager.resume_session(session_id).await;
        assert!(resume_result.is_ok(), "Session {} should support resume", i);
        
        let dtmf_result = manager.send_dtmf(session_id, "123").await;
        assert!(dtmf_result.is_ok(), "Session {} should support DTMF", i);
    }
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_session_capabilities_under_load() {
    let manager = create_capabilities_test_manager(true).await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create many sessions quickly
    let mut sessions = Vec::new();
    
    for i in 0..20 {
        let call = manager.create_outgoing_call(
            &format!("sip:load_caller_{}@example.com", i),
            &format!("sip:load_target_{}@example.com", i),
            Some(format!("Load test SDP {}", i))
        ).await.unwrap();
        sessions.push(call.id().clone());
        
        // Small delay to avoid overwhelming
        if i % 5 == 0 {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    }
    
    // Test capabilities on a subset of sessions
    for session_id in sessions.iter().take(5) {
        let hold_result = manager.hold_session(session_id).await;
        assert!(hold_result.is_ok());
        
        let resume_result = manager.resume_session(session_id).await;
        assert!(resume_result.is_ok());
    }
    
    // Clean up all sessions
    for session_id in sessions {
        let _ = manager.terminate_session(&session_id).await;
    }
    
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    let final_stats = manager.get_stats().await.unwrap();
    assert_eq!(final_stats.active_sessions, 0);
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_different_bind_address_configurations() {
    let configurations = vec![
        ("127.0.0.1", "sip:local@localhost"),
        ("0.0.0.0", "sip:any@example.com"),
    ];
    
    for (bind_addr, from_uri) in configurations {
        let manager = SessionManagerBuilder::new()
            .with_sip_bind_address(bind_addr)
            .with_sip_port(0) // Use any available port
            .with_from_uri(from_uri)
            .with_handler(Arc::new(CapabilitiesTestHandler::new(true)))
            .build()
            .await.unwrap();
        
        manager.start().await.unwrap();
        
        // Test basic capabilities with each configuration
        let call = manager.create_outgoing_call(
            from_uri,
            "sip:target@example.com",
            Some("Config test SDP".to_string())
        ).await.unwrap();
        
        let session_id = call.id().clone();
        let hold_result = manager.hold_session(&session_id).await;
        assert!(hold_result.is_ok());
        
        manager.stop().await.unwrap();
    }
}

#[tokio::test]
async fn test_error_handling_capabilities() {
    let manager = create_capabilities_test_manager(true).await.unwrap();
    
    manager.start().await.unwrap();
    
    let fake_session_id = SessionId::new();
    
    // Test that all operations gracefully handle non-existent sessions
    let hold_result = manager.hold_session(&fake_session_id).await;
    assert!(hold_result.is_err(), "Hold should return error for non-existent session");
    assert!(matches!(hold_result.unwrap_err(), SessionError::SessionNotFound(_)));
    
    let resume_result = manager.resume_session(&fake_session_id).await;
    assert!(resume_result.is_err(), "Resume should return error for non-existent session");
    
    let transfer_result = manager.transfer_session(&fake_session_id, "sip:target@example.com").await;
    assert!(transfer_result.is_err(), "Transfer should return error for non-existent session");
    
    let dtmf_result = manager.send_dtmf(&fake_session_id, "123").await;
    assert!(dtmf_result.is_err(), "DTMF should return error for non-existent session");
    
    let media_result = manager.update_media(&fake_session_id, "fake SDP").await;
    assert!(media_result.is_err(), "Media update should return error for non-existent session");
    
    let terminate_result = manager.terminate_session(&fake_session_id).await;
    assert!(terminate_result.is_err(), "Terminate should return error for non-existent session");
    
    manager.stop().await.unwrap();
} 