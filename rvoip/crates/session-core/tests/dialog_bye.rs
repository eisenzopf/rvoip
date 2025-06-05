//! Tests for BYE Dialog Integration
//!
//! Tests the session-core functionality for BYE requests (call termination),
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

/// Test handler for BYE testing
#[derive(Debug)]
struct ByeTestHandler {
    terminated_calls: Arc<tokio::sync::Mutex<Vec<(SessionId, String)>>>,
    call_durations: Arc<tokio::sync::Mutex<Vec<std::time::Duration>>>,
}

impl ByeTestHandler {
    fn new() -> Self {
        Self {
            terminated_calls: Arc::new(tokio::sync::Mutex::new(Vec::new())),
            call_durations: Arc::new(tokio::sync::Mutex::new(Vec::new())),
        }
    }

    async fn add_terminated_call(&self, session_id: SessionId, reason: String, duration: std::time::Duration) {
        self.terminated_calls.lock().await.push((session_id, reason));
        self.call_durations.lock().await.push(duration);
    }

    async fn get_terminated_calls(&self) -> Vec<(SessionId, String)> {
        self.terminated_calls.lock().await.clone()
    }

    async fn get_call_durations(&self) -> Vec<std::time::Duration> {
        self.call_durations.lock().await.clone()
    }
}

#[async_trait::async_trait]
impl CallHandler for ByeTestHandler {
    async fn on_incoming_call(&self, call: IncomingCall) -> CallDecision {
        CallDecision::Accept
    }

    async fn on_call_ended(&self, call: CallSession, reason: &str) {
        let duration = call.started_at
            .map(|start| start.elapsed())
            .unwrap_or_else(|| std::time::Duration::from_secs(0));
        
        self.add_terminated_call(call.id().clone(), reason.to_string(), duration).await;
        tracing::info!("BYE test call {} ended after {:?}: {}", call.id(), duration, reason);
    }
}

/// Create a test session manager for BYE testing
async fn create_bye_test_manager() -> Result<Arc<SessionManager>, SessionError> {
    let handler = Arc::new(ByeTestHandler::new());
    
    SessionManagerBuilder::new()
        .with_sip_bind_address("127.0.0.1")
        .with_sip_port(0) // Use any available port
        .with_from_uri("sip:test@localhost")
        .with_handler(handler)
        .build()
        .await
}

#[tokio::test]
async fn test_basic_bye_termination() {
    let manager = create_bye_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create call
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com", 
        Some("v=0\r\no=alice 123 456 IN IP4 192.168.1.100\r\nm=audio 5004 RTP/AVP 0\r\n".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Verify session exists before termination
    let session_before = manager.find_session(&session_id).await.unwrap();
    assert!(session_before.is_some());
    
    // Terminate with BYE
    let terminate_result = manager.terminate_session(&session_id).await;
    assert!(terminate_result.is_ok());
    
    // Wait for BYE processing
    tokio::time::sleep(Duration::from_millis(50)).await;
    
    // Verify session is removed after BYE
    let session_after = manager.find_session(&session_id).await.unwrap();
    assert!(session_after.is_none());
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_immediate_bye_after_invite() {
    let manager = create_bye_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create call and immediately terminate (fast BYE)
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("SDP offer".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Immediate termination (should send BYE or CANCEL depending on state)
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
async fn test_bye_after_call_establishment() {
    let manager = create_bye_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create call
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("SDP offer".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Simulate call being established (wait a bit)
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    // Now send BYE to terminate established call
    let terminate_result = manager.terminate_session(&session_id).await;
    assert!(terminate_result.is_ok());
    
    // Wait for BYE processing
    tokio::time::sleep(Duration::from_millis(50)).await;
    
    // Verify session cleanup
    let session_after = manager.find_session(&session_id).await.unwrap();
    assert!(session_after.is_none());
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_bye_multiple_concurrent_calls() {
    let manager = create_bye_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create multiple calls
    let mut calls = Vec::new();
    for i in 0..5 {
        let call = manager.create_outgoing_call(
            &format!("sip:caller{}@example.com", i),
            &format!("sip:target{}@example.com", i),
            Some(format!("SDP offer {}", i))
        ).await.unwrap();
        calls.push(call);
    }
    
    // Verify all calls exist
    let stats_before = manager.get_stats().await.unwrap();
    assert_eq!(stats_before.active_sessions, 5);
    
    // Terminate all calls with BYE
    for call in &calls {
        let result = manager.terminate_session(call.id()).await;
        assert!(result.is_ok());
    }
    
    // Wait for all BYE processing
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    // Verify all sessions are cleaned up
    let stats_after = manager.get_stats().await.unwrap();
    assert_eq!(stats_after.active_sessions, 0);
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_bye_nonexistent_session() {
    let manager = create_bye_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Try to send BYE to non-existent session
    let fake_session_id = SessionId::new();
    let terminate_result = manager.terminate_session(&fake_session_id).await;
    assert!(terminate_result.is_err());
    assert!(matches!(terminate_result.unwrap_err(), SessionError::SessionNotFound(_)));
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_bye_after_hold_operations() {
    let manager = create_bye_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create call
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("SDP offer".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Perform hold operations
    let hold_result = manager.hold_session(&session_id).await;
    assert!(hold_result.is_ok());
    
    let resume_result = manager.resume_session(&session_id).await;
    assert!(resume_result.is_ok());
    
    // Terminate after hold/resume sequence
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
async fn test_bye_after_media_updates() {
    let manager = create_bye_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create call
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("Initial SDP".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Perform media updates
    let update_result = manager.update_media(&session_id, "Updated SDP").await;
    assert!(update_result.is_ok());
    
    // Terminate after media update
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
async fn test_concurrent_bye_operations() {
    let manager = create_bye_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create multiple calls
    let mut calls = Vec::new();
    for i in 0..5 {
        let call = manager.create_outgoing_call(
            &format!("sip:caller{}@example.com", i),
            &format!("sip:target{}@example.com", i),
            Some(format!("BYE test SDP {}", i))
        ).await.unwrap();
        calls.push(call);
    }
    
    // Terminate all calls concurrently
    let mut bye_tasks = Vec::new();
    for call in calls {
        let manager_clone = manager.clone();
        let session_id = call.id().clone();
        let task = tokio::spawn(async move {
            manager_clone.terminate_session(&session_id).await
        });
        bye_tasks.push(task);
    }
    
    // Wait for all BYE operations to complete
    for task in bye_tasks {
        let result = task.await.unwrap();
        assert!(result.is_ok());
    }
    
    // Wait for cleanup
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    // Verify all sessions are cleaned up
    let final_stats = manager.get_stats().await.unwrap();
    assert_eq!(final_stats.active_sessions, 0);
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_bye_timing_measurements() {
    let manager = create_bye_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create call
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("SDP offer".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Wait to establish call duration
    tokio::time::sleep(Duration::from_millis(200)).await;
    
    // Measure BYE operation time
    let bye_start = std::time::Instant::now();
    let terminate_result = manager.terminate_session(&session_id).await;
    assert!(terminate_result.is_ok());
    let bye_duration = bye_start.elapsed();
    
    // BYE should complete quickly
    assert!(bye_duration < Duration::from_secs(1));
    
    // Wait for cleanup
    tokio::time::sleep(Duration::from_millis(50)).await;
    
    // Verify session cleanup
    let session_after = manager.find_session(&session_id).await.unwrap();
    assert!(session_after.is_none());
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_bye_session_state_transitions() {
    let manager = create_bye_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create call
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("SDP offer".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // Verify initial state
    let session_before = manager.find_session(&session_id).await.unwrap();
    assert!(session_before.is_some());
    assert_eq!(session_before.unwrap().state(), &CallState::Initiating);
    
    // Terminate session
    let terminate_result = manager.terminate_session(&session_id).await;
    assert!(terminate_result.is_ok());
    
    // Wait for state transition
    tokio::time::sleep(Duration::from_millis(50)).await;
    
    // Session should be removed (terminated)
    let session_after = manager.find_session(&session_id).await.unwrap();
    assert!(session_after.is_none());
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_double_bye_protection() {
    let manager = create_bye_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create call
    let call = manager.create_outgoing_call(
        "sip:alice@example.com",
        "sip:bob@example.com",
        Some("SDP offer".to_string())
    ).await.unwrap();
    
    let session_id = call.id().clone();
    
    // First BYE
    let first_bye = manager.terminate_session(&session_id).await;
    assert!(first_bye.is_ok());
    
    // Wait a moment
    tokio::time::sleep(Duration::from_millis(50)).await;
    
    // Second BYE (should fail gracefully)
    let second_bye = manager.terminate_session(&session_id).await;
    assert!(second_bye.is_err());
    assert!(matches!(second_bye.unwrap_err(), SessionError::SessionNotFound(_)));
    
    manager.stop().await.unwrap();
}

#[tokio::test]
async fn test_bye_statistics_tracking() {
    let manager = create_bye_test_manager().await.unwrap();
    
    manager.start().await.unwrap();
    
    // Create multiple calls
    let mut calls = Vec::new();
    for i in 0..3 {
        let call = manager.create_outgoing_call(
            &format!("sip:caller{}@example.com", i),
            &format!("sip:target{}@example.com", i),
            Some(format!("Stats test SDP {}", i))
        ).await.unwrap();
        calls.push(call);
    }
    
    // Verify initial stats
    let initial_stats = manager.get_stats().await.unwrap();
    assert_eq!(initial_stats.active_sessions, 3);
    
    // Terminate calls one by one and check stats
    for (i, call) in calls.iter().enumerate() {
        let result = manager.terminate_session(call.id()).await;
        assert!(result.is_ok());
        
        // Wait for cleanup
        tokio::time::sleep(Duration::from_millis(50)).await;
        
        // Check updated stats
        let current_stats = manager.get_stats().await.unwrap();
        assert_eq!(current_stats.active_sessions, 3 - (i + 1));
    }
    
    // Final verification
    let final_stats = manager.get_stats().await.unwrap();
    assert_eq!(final_stats.active_sessions, 0);
    
    manager.stop().await.unwrap();
} 