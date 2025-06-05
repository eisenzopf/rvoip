//! Tests for NOTIFY Dialog Integration
//!
//! Tests the session-core functionality for NOTIFY requests (event notifications),
//! ensuring proper integration with the underlying dialog layer.

use std::sync::Arc;
use std::time::Duration;
use rvoip_session_core::{
    SessionManager,
    SessionError,
    api::{
        types::{CallState, SessionId, IncomingCall, CallSession, CallDecision},
        handlers::CallHandler,
        builder::SessionManagerBuilder,
    },
};

/// Test handler for NOTIFY testing
#[derive(Debug)]
struct NotifyTestHandler {
    notify_events: Arc<tokio::sync::Mutex<Vec<(SessionId, String)>>>,
    subscriptions: Arc<tokio::sync::Mutex<Vec<String>>>,
}

impl NotifyTestHandler {
    fn new() -> Self {
        Self {
            notify_events: Arc::new(tokio::sync::Mutex::new(Vec::new())),
            subscriptions: Arc::new(tokio::sync::Mutex::new(Vec::new())),
        }
    }

    async fn add_notify_event(&self, session_id: SessionId, event: String) {
        self.notify_events.lock().await.push((session_id, event));
    }

    async fn get_notify_events(&self) -> Vec<(SessionId, String)> {
        self.notify_events.lock().await.clone()
    }

    async fn add_subscription(&self, event_package: String) {
        self.subscriptions.lock().await.push(event_package);
    }

    async fn get_subscriptions(&self) -> Vec<String> {
        self.subscriptions.lock().await.clone()
    }
}

#[async_trait::async_trait]
impl CallHandler for NotifyTestHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        CallDecision::Accept
    }

    async fn on_call_ended(&self, call: CallSession, reason: &str) {
        if reason.contains("notify") || reason.contains("NOTIFY") {
            self.add_notify_event(call.id().clone(), reason.to_string()).await;
        }
        tracing::info!("NOTIFY test call {} ended: {}", call.id(), reason);
    }
}

/// Create a test session manager for NOTIFY testing
async fn create_notify_test_manager() -> Result<Arc<SessionManager>, SessionError> {
    let handler = Arc::new(NotifyTestHandler::new());
    
    SessionManagerBuilder::new()
        .with_sip_bind_address("127.0.0.1")
        .with_sip_port(0) // Use any available port
        .with_from_uri("sip:test@localhost")
        .with_handler(handler)
        .build()
        .await
}

#[tokio::test]
async fn test_session_with_notify_support() {
    let manager = create_notify_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create a session that might send/receive NOTIFY messages
    let result = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com", 
        Some("v=0\r\no=alice 123 456 IN IP4 192.168.1.100\r\nm=audio 5004 RTP/AVP 0\r\n".to_string())
    ).await;
    
    assert!(result.is_ok());
    let call = result.unwrap();
    assert_eq!(call.state(), &CallState::Initiating);
    
    // In real scenario, NOTIFY would be sent/received through dialog-core
    // Here we're testing that session-core can handle NOTIFY-related operations
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_notify_for_call_state_changes() {
    let manager = create_notify_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create call
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("SDP offer".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Test operations that might trigger NOTIFY messages
    
    // Hold operation (might send NOTIFY with dialog state)
    let hold_result = manager.hold_session(&session_id).await;
    assert!(hold_result.is_ok());
    
    // Resume operation (might send NOTIFY with dialog state)
    let resume_result = manager.resume_session(&session_id).await;
    assert!(resume_result.is_ok());
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_notify_for_transfer_events() {
    let manager = create_notify_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create call
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("SDP offer".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Transfer operation (should trigger NOTIFY messages for transfer status)
    let transfer_result = manager.transfer_session(&session_id, "sip:charlie@example.com").await;
    assert!(transfer_result.is_ok());
    
    // In real implementation, this would send REFER and receive NOTIFY messages
    // about transfer progress
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_notify_message_info_integration() {
    let manager = create_notify_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create call
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("SDP offer".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Send INFO that might be related to NOTIFY events
    let info_result = manager.send_dtmf(&session_id, "123").await;
    assert!(info_result.is_ok());
    
    // In real scenario, DTMF might be sent via INFO and status via NOTIFY
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_multiple_notify_subscriptions() {
    let manager = create_notify_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create multiple calls that might have different NOTIFY subscriptions
    let mut calls = Vec::new();
    for i in 0..3 {
        let call = manager.create_outgoing_call(
            &format!("sip:caller{}@example.com", i),
            &format!("sip:target{}@example.com", i),
            Some(format!("SDP for NOTIFY test {}", i))
        ).await.unwrap();
        calls.push(call);
    }
    
    // Each call might have different NOTIFY event packages
    for call in &calls {
        let session = manager.find_session(call.id()).await.unwrap();
        assert!(session.is_some());
    }
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_notify_error_handling() {
    let manager = create_notify_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Try NOTIFY-related operations on non-existent session
    let fake_session_id = SessionId::new();
    
    // Operations that might involve NOTIFY should fail appropriately
    let transfer_result = manager.transfer_session(&fake_session_id, "sip:target@example.com").await;
    assert!(transfer_result.is_err());
    assert!(matches!(transfer_result.unwrap_err(), SessionError::SessionNotFound(_)));
    
    let dtmf_result = manager.send_dtmf(&fake_session_id, "123").await;
    assert!(dtmf_result.is_err());
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_notify_event_sequencing() {
    let manager = create_notify_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create call
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("SDP offer".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Test sequence of operations that might generate NOTIFY events
    let hold_result = manager.hold_session(&session_id).await;
    assert!(hold_result.is_ok());
    
    tokio::time::sleep(Duration::from_millis(10)).await;
    
    let resume_result = manager.resume_session(&session_id).await;
    assert!(resume_result.is_ok());
    
    tokio::time::sleep(Duration::from_millis(10)).await;
    
    let transfer_result = manager.transfer_session(&session_id, "sip:transfer@example.com").await;
    assert!(transfer_result.is_ok());
    
    // Each operation should maintain proper NOTIFY event sequencing
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_notify_session_state_consistency() {
    let manager = create_notify_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create call
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("Initial SDP".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Verify session exists before NOTIFY operations
    let session_before = manager.find_session(&session_id).await.unwrap();
    assert!(session_before.is_some());
    
    // Operations that might involve NOTIFY
    let hold_result = manager.hold_session(&session_id).await;
    assert!(hold_result.is_ok());
    
    // Verify session consistency after NOTIFY-triggering operations
    let session_after = manager.find_session(&session_id).await.unwrap();
    assert!(session_after.is_some());
    assert_eq!(session_after.unwrap().id(), &session_id);
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_concurrent_notify_operations() {
    let manager = create_notify_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create multiple calls
    let mut calls = Vec::new();
    for i in 0..5 {
        let call = manager.create_outgoing_call(
            &format!("sip:caller{}@example.com", i),
            &format!("sip:target{}@example.com", i),
            Some(format!("NOTIFY test SDP {}", i))
        ).await.unwrap();
        calls.push(call);
    }
    
    // Perform concurrent operations that might trigger NOTIFY
    let mut tasks = Vec::new();
    for (i, call) in calls.iter().enumerate() {
        let manager_clone = manager.clone();
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
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_notify_subscription_lifecycle() {
    let manager = create_notify_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create call (might establish NOTIFY subscriptions)
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("SDP offer".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Session operations that might affect NOTIFY subscriptions
    let hold_result = manager.hold_session(&session_id).await;
    assert!(hold_result.is_ok());
    
    // Terminate session (should properly clean up NOTIFY subscriptions)
    let terminate_result = manager.terminate_session(&session_id).await;
    assert!(terminate_result.is_ok());
    
    // Wait for cleanup
    tokio::time::sleep(Duration::from_millis(50)).await;
    
    // Verify session cleanup
    let session_after = manager.find_session(&session_id).await.unwrap();
    assert!(session_after.is_none());
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_notify_timing_and_expiration() {
    let manager = create_notify_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create call
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("SDP offer".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Test rapid operations that might generate NOTIFY events
    let start_time = std::time::Instant::now();
    
    for i in 0..3 {
        if i % 2 == 0 {
            let _ = manager.hold_session(&session_id).await;
        } else {
            let _ = manager.resume_session(&session_id).await;
        }
        tokio::time::sleep(Duration::from_millis(5)).await;
    }
    
    let elapsed = start_time.elapsed();
    // NOTIFY-related operations should complete quickly
    assert!(elapsed < Duration::from_secs(1));
    
    manager.stop().await.unwrap();
} 