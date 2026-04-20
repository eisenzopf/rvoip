//! Tests for the simple API (v3)
//!
//! These tests demonstrate the StreamPeer and SimplePeer API usage.
//! Most tests that relied on old SimplePeer methods (hold, resume, send_dtmf,
//! transfer, recording, conference, incoming_call, wait_for_call, subscribe_audio,
//! send_audio, reject) have been migrated to use the StreamPeer / UnifiedCoordinator
//! APIs.

use rvoip_session_core::api::unified::Config;
use rvoip_session_core::StreamPeer;
use rvoip_session_core::SessionId;
use std::time::Duration;
use tokio::time::timeout;
use serial_test::serial;

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
        use_100rel: Default::default(),
        session_timer_secs: None,
        session_timer_min_se: 90,
        credentials: None,
    }
}

#[tokio::test]
#[serial]
async fn test_create_peer() {
    let peer = StreamPeer::new("alice").await;
    assert!(peer.is_ok());
}

#[tokio::test]
#[serial]
async fn test_make_outgoing_call() {
    let mut peer = StreamPeer::with_config(test_config(15100)).await.unwrap();

    // Make a call - returns a SessionHandle
    let handle = peer.call("sip:bob@localhost:15101").await;
    assert!(handle.is_ok());
}

#[tokio::test]
#[serial]
async fn test_hold_resume_call_via_coordinator() {
    // Hold/resume is available via UnifiedCoordinator
    let config = test_config(15102);
    let coordinator = rvoip_session_core::UnifiedCoordinator::new(config).await.unwrap();

    // Make a call
    let session_id = coordinator.make_call(
        "sip:alice@localhost",
        "sip:bob@localhost:15103",
    ).await.unwrap();

    // Put on hold
    let hold_result = coordinator.hold(&session_id).await;
    assert!(hold_result.is_ok());

    // Resume
    let resume_result = coordinator.resume(&session_id).await;
    assert!(resume_result.is_ok());
}

#[tokio::test]
#[serial]
async fn test_send_dtmf_via_coordinator() {
    let config = test_config(15104);
    let coordinator = rvoip_session_core::UnifiedCoordinator::new(config).await.unwrap();

    // Make a call
    let session_id = coordinator.make_call(
        "sip:alice@localhost",
        "sip:bob@localhost:15105",
    ).await.unwrap();

    // Send DTMF digits
    assert!(coordinator.send_dtmf(&session_id, '1').await.is_ok());
    assert!(coordinator.send_dtmf(&session_id, '2').await.is_ok());
    assert!(coordinator.send_dtmf(&session_id, '3').await.is_ok());
    assert!(coordinator.send_dtmf(&session_id, '#').await.is_ok());
}

// Blind transfer needs a real peer on the wire, not an in-process neighbour —
// running two StreamPeers in the same Tokio runtime creates socket/state
// collisions we've hit repeatedly. Transfer coverage lives in the multi-binary
// integration test `tests/blind_transfer_integration.rs`, which launches Alice,
// Bob, and Charlie as separate processes.

#[tokio::test]
#[serial]
async fn test_recording_via_coordinator() {
    let config = test_config(15109);
    let coordinator = rvoip_session_core::UnifiedCoordinator::new(config).await.unwrap();

    // Make a call
    let session_id = coordinator.make_call(
        "sip:alice@localhost",
        "sip:bob@localhost:15110",
    ).await.unwrap();

    // Start recording
    assert!(coordinator.start_recording(&session_id).await.is_ok());

    // Stop recording
    assert!(coordinator.stop_recording(&session_id).await.is_ok());
}

#[tokio::test]
#[serial]
async fn test_conference_creation_via_coordinator() {
    let config = test_config(15111);
    let coordinator = rvoip_session_core::UnifiedCoordinator::new(config).await.unwrap();

    // Make first call
    let call1 = coordinator.make_call(
        "sip:alice@localhost",
        "sip:bob@localhost:15112",
    ).await.unwrap();

    // Create conference from the call
    let conf_result = coordinator.create_conference(&call1, "Test Conference").await;
    assert!(conf_result.is_ok());

    // Make second call
    let call2 = coordinator.make_call(
        "sip:alice@localhost",
        "sip:charlie@localhost:15113",
    ).await.unwrap();

    // Add second call to conference
    let add_result = coordinator.add_to_conference(&call1, &call2).await;
    assert!(add_result.is_ok());
}

#[tokio::test]
#[serial]
async fn test_wait_for_incoming_with_timeout() {
    let mut peer = StreamPeer::with_config(test_config(15115)).await.unwrap();

    // Wait for incoming call with timeout - should timeout since no caller
    let wait_result = timeout(
        Duration::from_millis(100),
        peer.wait_for_incoming(),
    ).await;

    // Should timeout since no incoming call
    assert!(wait_result.is_err());
}

#[tokio::test]
#[serial]
async fn test_accept_reject_incoming_via_coordinator() {
    let config = test_config(15116);
    let coordinator = rvoip_session_core::UnifiedCoordinator::new(config).await.unwrap();

    // Simulate accepting a call (would need real session ID)
    let fake_session_id = SessionId::new();
    let accept_result = coordinator.accept_call(&fake_session_id).await;
    // Will fail because session doesn't exist, but API works
    assert!(accept_result.is_err());

    // Simulate rejecting a call
    let reject_result = coordinator.reject_call(&fake_session_id, 486, "Busy").await;
    // Will fail because session doesn't exist, but API works
    assert!(reject_result.is_err());
}

#[tokio::test]
#[serial]
async fn test_hangup_call() {
    let mut peer = StreamPeer::with_config(test_config(15117)).await.unwrap();

    // Make a call
    let handle = peer.call("sip:bob@localhost:15118").await.unwrap();

    // Hang up via SessionHandle
    let hangup_result = handle.hangup().await;
    assert!(hangup_result.is_ok());
}

// Integration test with two peers
#[tokio::test]
#[serial]
async fn test_peer_to_peer_call() {
    // Create two peers
    let mut alice = StreamPeer::with_config(test_config(15119)).await.unwrap();
    let _bob = StreamPeer::with_config(test_config(15120)).await.unwrap();

    // Alice calls Bob
    let handle = alice.call("sip:bob@localhost:15120").await.unwrap();

    // Alice hangs up
    assert!(handle.hangup().await.is_ok());
}
