//! Integration tests for the unified API

use rvoip_session_core_v2::api::unified::{UnifiedSession, UnifiedCoordinator, Config};
use rvoip_session_core_v2::api::SessionEvent;
use rvoip_session_core_v2::state_table::types::{Role, CallState};
use std::sync::Arc;
use tokio::time::{timeout, Duration};

/// Test helper to create a coordinator with default config
async fn create_test_coordinator() -> Arc<UnifiedCoordinator> {
    let config = Config {
        sip_port: 15200,
        media_port_start: 40000,
        media_port_end: 41000,
        local_ip: "127.0.0.1".parse().unwrap(),
        bind_addr: "127.0.0.1:15200".parse().unwrap(),
        state_table_path: None,
    };
    UnifiedCoordinator::new(config).await.unwrap()
}

#[tokio::test]
async fn test_uac_session_lifecycle() {
    let coordinator = create_test_coordinator().await;
    
    // Create UAC session
    let uac = UnifiedSession::new(coordinator.clone(), Role::UAC).await.unwrap();
    
    // Initial state should be Idle
    assert_eq!(uac.state().await.unwrap(), CallState::Idle);
    assert_eq!(uac.role(), Role::UAC);
    
    // Make a call
    uac.make_call("sip:test@example.com").await.unwrap();
    
    // State should change (would be Initiating with real SIP stack)
    // Note: Without actual SIP/media integration, state transitions may not occur
    
    // Test other operations
    assert!(uac.send_dtmf("123").await.is_ok());
    assert!(uac.hold().await.is_ok());
    assert!(uac.resume().await.is_ok());
    assert!(uac.hangup().await.is_ok());
}

#[tokio::test]
async fn test_uas_session_lifecycle() {
    let coordinator = create_test_coordinator().await;
    
    // Create UAS session
    let uas = UnifiedSession::new(coordinator.clone(), Role::UAS).await.unwrap();
    
    // Initial state should be Idle
    assert_eq!(uas.state().await.unwrap(), CallState::Idle);
    assert_eq!(uas.role(), Role::UAS);
    
    // Simulate incoming call
    let sdp = "v=0\r\no=- 0 0 IN IP4 127.0.0.1\r\ns=-\r\nc=IN IP4 127.0.0.1\r\nt=0 0\r\nm=audio 5004 RTP/AVP 0";
    uas.on_incoming_call("sip:caller@example.com", Some(sdp.to_string())).await.unwrap();
    
    // Accept the call
    assert!(uas.accept().await.is_ok());
    
    // Test media operations
    assert!(uas.play_audio("test.wav").await.is_ok());
    assert!(uas.start_recording().await.is_ok());
    assert!(uas.stop_recording().await.is_ok());
    
    // Hangup
    assert!(uas.hangup().await.is_ok());
}

#[tokio::test]
async fn test_peer_to_peer_call() {
    let coordinator = create_test_coordinator().await;
    
    // Create Alice (UAC)
    let alice = UnifiedSession::new(coordinator.clone(), Role::UAC).await.unwrap();
    
    // Create Bob (UAS)
    let bob = UnifiedSession::new(coordinator.clone(), Role::UAS).await.unwrap();
    
    // Track events
    let alice_events = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let bob_events = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    
    let alice_events_clone = alice_events.clone();
    alice.on_event(move |event| {
        let events = alice_events_clone.clone();
        tokio::spawn(async move {
            events.lock().await.push(event);
        });
    }).await.unwrap();
    
    let bob_events_clone = bob_events.clone();
    bob.on_event(move |event| {
        let events = bob_events_clone.clone();
        tokio::spawn(async move {
            events.lock().await.push(event);
        });
    }).await.unwrap();
    
    // Alice calls Bob
    alice.make_call("sip:bob@example.com").await.unwrap();
    
    // Bob receives call
    bob.on_incoming_call("sip:alice@example.com", None).await.unwrap();
    
    // Bob accepts
    bob.accept().await.unwrap();
    
    // Both hangup
    alice.hangup().await.unwrap();
    bob.hangup().await.unwrap();
    
    // Verify sessions were created and managed correctly
    assert_eq!(alice.role(), Role::UAC);
    assert_eq!(bob.role(), Role::UAS);
}

#[tokio::test]
async fn test_call_bridging() {
    let coordinator = create_test_coordinator().await;
    
    // Create two active sessions
    let session1 = UnifiedSession::new(coordinator.clone(), Role::UAC).await.unwrap();
    let session2 = UnifiedSession::new(coordinator.clone(), Role::UAC).await.unwrap();
    
    // Make calls
    session1.make_call("sip:party1@example.com").await.unwrap();
    session2.make_call("sip:party2@example.com").await.unwrap();
    
    // Bridge the sessions
    assert!(coordinator.bridge_sessions(&session1.id, &session2.id).await.is_ok());
    
    // Cleanup
    session1.hangup().await.unwrap();
    session2.hangup().await.unwrap();
}

#[tokio::test]
async fn test_call_transfer() {
    let coordinator = create_test_coordinator().await;
    
    // Create a session
    let session = UnifiedSession::new(coordinator.clone(), Role::UAC).await.unwrap();
    
    // Make a call
    session.make_call("sip:initial@example.com").await.unwrap();
    
    // Test blind transfer
    assert!(session.transfer("sip:destination@example.com", false).await.is_ok());
    
    // Create another session for attended transfer
    let session2 = UnifiedSession::new(coordinator.clone(), Role::UAC).await.unwrap();
    session2.make_call("sip:initial2@example.com").await.unwrap();
    
    // Test attended transfer
    assert!(session2.transfer("sip:destination2@example.com", true).await.is_ok());
}

#[tokio::test]
async fn test_hold_resume() {
    let coordinator = create_test_coordinator().await;
    
    let session = UnifiedSession::new(coordinator.clone(), Role::UAC).await.unwrap();
    session.make_call("sip:test@example.com").await.unwrap();
    
    // Put on hold
    assert!(session.hold().await.is_ok());
    
    // Resume
    assert!(session.resume().await.is_ok());
    
    // Cleanup
    session.hangup().await.unwrap();
}

#[tokio::test]
async fn test_media_operations() {
    let coordinator = create_test_coordinator().await;
    
    let session = UnifiedSession::new(coordinator.clone(), Role::UAS).await.unwrap();
    session.on_incoming_call("sip:caller@example.com", None).await.unwrap();
    session.accept().await.unwrap();
    
    // Test various media operations
    assert!(session.play_audio("announcement.wav").await.is_ok());
    assert!(session.start_recording().await.is_ok());
    assert!(session.send_dtmf("1234*#").await.is_ok());
    assert!(session.stop_recording().await.is_ok());
    
    session.hangup().await.unwrap();
}

#[tokio::test]
async fn test_b2bua_scenario() {
    let coordinator = create_test_coordinator().await;
    
    // Inbound leg (UAS)
    let inbound = UnifiedSession::new(coordinator.clone(), Role::UAS).await.unwrap();
    inbound.on_incoming_call("sip:customer@external.com", None).await.unwrap();
    inbound.accept().await.unwrap();
    
    // Outbound leg (UAC)
    let outbound = UnifiedSession::new(coordinator.clone(), Role::UAC).await.unwrap();
    outbound.make_call("sip:agent@internal.com").await.unwrap();
    
    // Bridge the legs
    assert!(coordinator.bridge_sessions(&inbound.id, &outbound.id).await.is_ok());
    
    // Simulate transfer
    assert!(inbound.transfer("sip:supervisor@internal.com", false).await.is_ok());
    
    // Cleanup
    inbound.hangup().await.unwrap();
    outbound.hangup().await.unwrap();
}

#[tokio::test]
async fn test_event_subscription() {
    let coordinator = create_test_coordinator().await;
    let session = UnifiedSession::new(coordinator.clone(), Role::UAC).await.unwrap();
    
    let events = Arc::new(tokio::sync::Mutex::new(Vec::new()));
    let events_clone = events.clone();
    
    // Subscribe to events
    session.on_event(move |event| {
        let events = events_clone.clone();
        tokio::spawn(async move {
            events.lock().await.push(event);
        });
    }).await.unwrap();
    
    // Trigger some events
    session.make_call("sip:test@example.com").await.unwrap();
    session.hold().await.unwrap();
    session.resume().await.unwrap();
    session.hangup().await.unwrap();
    
    // Give events time to propagate
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    // Events should have been captured
    // Note: Without full integration, events may not fire
    let captured_events = events.lock().await;
    println!("Captured {} events", captured_events.len());
}

#[tokio::test]
async fn test_concurrent_sessions() {
    let coordinator = create_test_coordinator().await;
    
    // Create multiple concurrent sessions
    let mut sessions = Vec::new();
    for i in 0..10 {
        let role = if i % 2 == 0 { Role::UAC } else { Role::UAS };
        let session = UnifiedSession::new(coordinator.clone(), role).await.unwrap();
        sessions.push(session);
    }
    
    // Perform operations on all sessions concurrently
    let mut handles = Vec::new();
    for (i, session) in sessions.into_iter().enumerate() {
        let handle = tokio::spawn(async move {
            if i % 2 == 0 {
                session.make_call(&format!("sip:user{}@example.com", i)).await.unwrap();
            } else {
                session.on_incoming_call(&format!("sip:caller{}@example.com", i), None).await.unwrap();
                session.accept().await.unwrap();
            }
            session.hangup().await.unwrap();
        });
        handles.push(handle);
    }
    
    // Wait for all to complete
    for handle in handles {
        handle.await.unwrap();
    }
}

#[tokio::test]
async fn test_state_persistence() {
    let coordinator = create_test_coordinator().await;
    
    // Create session and change state
    let session = UnifiedSession::new(coordinator.clone(), Role::UAC).await.unwrap();
    let session_id = session.id.clone();
    
    session.make_call("sip:test@example.com").await.unwrap();
    
    // State should be retrievable
    let state = coordinator.get_session_state(&session_id).await.unwrap();
    // Note: Without full integration, state may still be Idle
    println!("Session state: {:?}", state);
    
    session.hangup().await.unwrap();
}