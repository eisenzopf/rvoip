//! Tests for the unified API
//! 
//! These tests demonstrate the unified coordinator API usage

use rvoip_session_core_v2::api::unified::{UnifiedCoordinator, Config};
use rvoip_session_core_v2::state_table::types::SessionId;
use rvoip_session_core_v2::types::CallState;
use std::time::Duration;
use tokio::time::timeout;

/// Create a test configuration with unique ports
fn test_config(base_port: u16) -> Config {
    Config {
        sip_port: base_port,
        media_port_start: base_port + 1000,
        media_port_end: base_port + 2000,
        local_ip: "127.0.0.1".parse().unwrap(),
        bind_addr: format!("127.0.0.1:{}", base_port).parse().unwrap(),
        state_table_path: None,
        local_uri: format!("sip:test@127.0.0.1:{}", base_port),
    }
}

#[tokio::test]
async fn test_create_coordinator() {
    let coordinator = UnifiedCoordinator::new(test_config(15200)).await;
    assert!(coordinator.is_ok());
}

#[tokio::test]
async fn test_make_call() {
    let coordinator = UnifiedCoordinator::new(test_config(15201)).await.unwrap();
    
    let session_id = coordinator.make_call(
        "sip:alice@localhost",
        "sip:bob@localhost:15202"
    ).await;
    
    assert!(session_id.is_ok());
    let session_id = session_id.unwrap();
    
    // Check state
    let state = coordinator.get_state(&session_id).await;
    assert!(state.is_ok());
    // Should be Initiating
    assert_eq!(state.unwrap(), CallState::Initiating);
}

#[tokio::test]
async fn test_hold_resume() {
    let coordinator = UnifiedCoordinator::new(test_config(15203)).await.unwrap();
    
    let session_id = coordinator.make_call(
        "sip:alice@localhost",
        "sip:bob@localhost:15204"
    ).await.unwrap();
    
    // Hold
    let hold_result = coordinator.hold(&session_id).await;
    assert!(hold_result.is_ok());
    
    // Resume
    let resume_result = coordinator.resume(&session_id).await;
    assert!(resume_result.is_ok());
}

#[tokio::test]
async fn test_conference_operations() {
    let coordinator = UnifiedCoordinator::new(test_config(15205)).await.unwrap();
    
    // Create first call
    let session1 = coordinator.make_call(
        "sip:alice@localhost",
        "sip:bob@localhost:15206"
    ).await.unwrap();
    
    // Create conference from first call
    let conf_result = coordinator.create_conference(&session1, "Board Meeting").await;
    assert!(conf_result.is_ok());
    
    // Create second call
    let session2 = coordinator.make_call(
        "sip:alice@localhost",
        "sip:charlie@localhost:15207"
    ).await.unwrap();
    
    // Add to conference
    let add_result = coordinator.add_to_conference(&session1, &session2).await;
    assert!(add_result.is_ok());
    
    // Check if in conference
    let in_conf1 = coordinator.is_in_conference(&session1).await;
    assert!(in_conf1.is_ok());
}

#[tokio::test]
async fn test_transfer_operations() {
    let coordinator = UnifiedCoordinator::new(test_config(15208)).await.unwrap();
    
    let session_id = coordinator.make_call(
        "sip:alice@localhost",
        "sip:bob@localhost:15209"
    ).await.unwrap();
    
    // Blind transfer
    let blind_result = coordinator.blind_transfer(
        &session_id,
        "sip:charlie@localhost:15210"
    ).await;
    assert!(blind_result.is_ok());
}

#[tokio::test]
async fn test_attended_transfer() {
    let coordinator = UnifiedCoordinator::new(test_config(15211)).await.unwrap();
    
    let session_id = coordinator.make_call(
        "sip:alice@localhost",
        "sip:bob@localhost:15212"
    ).await.unwrap();
    
    // Start attended transfer
    let start_result = coordinator.start_attended_transfer(
        &session_id,
        "sip:charlie@localhost:15213"
    ).await;
    assert!(start_result.is_ok());
    
    // Complete attended transfer
    let complete_result = coordinator.complete_attended_transfer(&session_id).await;
    assert!(complete_result.is_ok());
}

#[tokio::test]
async fn test_dtmf_operations() {
    let coordinator = UnifiedCoordinator::new(test_config(15214)).await.unwrap();
    
    let session_id = coordinator.make_call(
        "sip:alice@localhost",
        "sip:bob@localhost:15215"
    ).await.unwrap();
    
    // Send DTMF digits
    for digit in "1234567890*#".chars() {
        let result = coordinator.send_dtmf(&session_id, digit).await;
        assert!(result.is_ok());
    }
}

#[tokio::test]
async fn test_recording_operations() {
    let coordinator = UnifiedCoordinator::new(test_config(15216)).await.unwrap();
    
    let session_id = coordinator.make_call(
        "sip:alice@localhost",
        "sip:bob@localhost:15217"
    ).await.unwrap();
    
    // Start recording
    let start_result = coordinator.start_recording(&session_id).await;
    assert!(start_result.is_ok());
    
    // Stop recording
    let stop_result = coordinator.stop_recording(&session_id).await;
    assert!(stop_result.is_ok());
}

#[tokio::test]
async fn test_session_queries() {
    let coordinator = UnifiedCoordinator::new(test_config(15218)).await.unwrap();
    
    // List sessions (should be empty)
    let sessions = coordinator.list_sessions().await;
    assert_eq!(sessions.len(), 0);
    
    // Make a call
    let session_id = coordinator.make_call(
        "sip:alice@localhost",
        "sip:bob@localhost:15219"
    ).await.unwrap();
    
    // List sessions (should have one)
    let sessions = coordinator.list_sessions().await;
    assert_eq!(sessions.len(), 1);
    assert_eq!(sessions[0].session_id, session_id);
    
    // Get session info
    let info = coordinator.get_session_info(&session_id).await;
    assert!(info.is_ok());
    let info = info.unwrap();
    assert_eq!(info.from, "sip:alice@localhost");
    assert_eq!(info.to, "sip:bob@localhost:15219");
}

#[tokio::test]
async fn test_event_subscription() {
    let coordinator = UnifiedCoordinator::new(test_config(15220)).await.unwrap();
    
    let session_id = coordinator.make_call(
        "sip:alice@localhost",
        "sip:bob@localhost:15221"
    ).await.unwrap();
    
    // Subscribe to events
    let (tx, mut rx) = tokio::sync::mpsc::channel(10);
    coordinator.subscribe(session_id.clone(), move |event| {
        let _ = tx.try_send(event);
    }).await;
    
    // Hangup to generate an event
    let _ = coordinator.hangup(&session_id).await;
    
    // Should receive event (with timeout)
    let _event = timeout(Duration::from_millis(100), rx.recv()).await;
    // Event system is async, may or may not receive immediately
    
    // Unsubscribe
    coordinator.unsubscribe(&session_id).await;
}

#[tokio::test]
async fn test_accept_reject_calls() {
    let coordinator = UnifiedCoordinator::new(test_config(15222)).await.unwrap();
    
    // These will fail without actual incoming calls, but test the API
    let fake_session_id = SessionId::new();
    
    // Accept
    let accept_result = coordinator.accept_call(&fake_session_id).await;
    assert!(accept_result.is_err()); // No such session
    
    // Reject
    let reject_result = coordinator.reject_call(&fake_session_id, "Busy").await;
    assert!(reject_result.is_err()); // No such session
}

#[tokio::test]
async fn test_multiple_calls() {
    let coordinator = UnifiedCoordinator::new(test_config(15223)).await.unwrap();
    
    // Make multiple calls
    let session1 = coordinator.make_call(
        "sip:alice@localhost",
        "sip:bob@localhost:15224"
    ).await.unwrap();
    
    let session2 = coordinator.make_call(
        "sip:alice@localhost",
        "sip:charlie@localhost:15225"
    ).await.unwrap();
    
    let session3 = coordinator.make_call(
        "sip:alice@localhost",
        "sip:david@localhost:15226"
    ).await.unwrap();
    
    // List all sessions
    let sessions = coordinator.list_sessions().await;
    assert_eq!(sessions.len(), 3);
    
    // Hangup all
    assert!(coordinator.hangup(&session1).await.is_ok());
    assert!(coordinator.hangup(&session2).await.is_ok());
    assert!(coordinator.hangup(&session3).await.is_ok());
}
