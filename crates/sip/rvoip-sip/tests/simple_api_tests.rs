//! Tests for the simple API (v3)
//!
//! These tests demonstrate the StreamPeer and SimplePeer API usage.
//! Most tests that relied on old SimplePeer methods (hold, resume, send_dtmf,
//! transfer, recording, conference, incoming_call, wait_for_call, subscribe_audio,
//! send_audio, reject) have been migrated to use the StreamPeer / UnifiedCoordinator
//! APIs.

mod support;

use rvoip_sip::api::unified::Config;
use rvoip_sip::SessionId;
use rvoip_sip::StreamPeer;
use serial_test::serial;
use std::time::Duration;
use support::free_udp_ports;
use tokio::time::timeout;

/// Create a test configuration on a port the kernel reports free.
///
/// The media range is derived from a second free port rather than from the SIP
/// port, so a busy neighbour on `sip_port + 1000` cannot break the range.
fn test_config() -> Config {
    let [sip_port, media_base] = free_udp_ports::<2>();
    let mut config = Config::local("test", sip_port);
    config.media_port_start = media_base;
    config.media_port_end = media_base + 100;
    config
}

/// `StreamPeer::new` is the single-instance convenience constructor: it takes
/// `Config::default()`, which binds the SIP well-known port 5060.
///
/// That port cannot be assumed free — on a SIP developer's machine it is the
/// one most likely to be taken, and two `new` peers in one process would
/// collide with each other anyway. So the binding half is exercised through
/// `with_config` on a free port, and the part `new` uniquely owns, the default
/// configuration it derives, is asserted without binding anything.
#[tokio::test]
#[serial]
async fn test_create_peer() {
    let defaults = Config::default();
    assert_eq!(defaults.sip_port, 5060);
    assert_eq!(defaults.bind_addr.port(), 5060);

    let peer = StreamPeer::with_config(test_config()).await;
    assert!(peer.is_ok());
}

#[tokio::test]
#[serial]
async fn test_make_outgoing_call() {
    let peer = StreamPeer::with_config(test_config()).await.unwrap();

    // Make a call - returns a SessionHandle
    let handle = peer.invite("sip:bob@localhost:15101").send().await;
    assert!(handle.is_ok());
}

#[tokio::test]
#[serial]
async fn test_hold_resume_call_via_coordinator() {
    // Hold/resume is available via UnifiedCoordinator
    let config = test_config();
    let coordinator = rvoip_sip::UnifiedCoordinator::new(config).await.unwrap();

    // Make a call
    let session_id = coordinator
        .invite(
            Some("sip:alice@localhost".to_string()),
            "sip:bob@localhost:15103",
        )
        .send()
        .await
        .unwrap();

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
    let config = test_config();
    let coordinator = rvoip_sip::UnifiedCoordinator::new(config).await.unwrap();

    // Make a call
    let session_id = coordinator
        .invite(
            Some("sip:alice@localhost".to_string()),
            "sip:bob@localhost:15105",
        )
        .send()
        .await
        .unwrap();

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
    let config = test_config();
    let coordinator = rvoip_sip::UnifiedCoordinator::new(config).await.unwrap();

    // Make a call
    let session_id = coordinator
        .invite(
            Some("sip:alice@localhost".to_string()),
            "sip:bob@localhost:15110",
        )
        .send()
        .await
        .unwrap();

    // Start recording
    assert!(coordinator.start_recording(&session_id).await.is_ok());

    // Stop recording
    assert!(coordinator.stop_recording(&session_id).await.is_ok());
}

#[tokio::test]
#[serial]
async fn test_conference_creation_via_coordinator() {
    let config = test_config();
    let coordinator = rvoip_sip::UnifiedCoordinator::new(config).await.unwrap();

    // Make first call
    let call1 = coordinator
        .invite(
            Some("sip:alice@localhost".to_string()),
            "sip:bob@localhost:15112",
        )
        .send()
        .await
        .unwrap();

    // Create conference from the call
    let conf_result = coordinator
        .create_conference(&call1, "Test Conference")
        .await;
    assert!(conf_result.is_ok());

    // Make second call
    let call2 = coordinator
        .invite(
            Some("sip:alice@localhost".to_string()),
            "sip:charlie@localhost:15113",
        )
        .send()
        .await
        .unwrap();

    // Add second call to conference
    let add_result = coordinator.add_to_conference(&call1, &call2).await;
    assert!(add_result.is_ok());
}

#[tokio::test]
#[serial]
async fn test_wait_for_incoming_with_timeout() {
    let mut peer = StreamPeer::with_config(test_config()).await.unwrap();

    // Wait for incoming call with timeout - should timeout since no caller
    let wait_result = timeout(Duration::from_millis(100), peer.wait_for_incoming()).await;

    // Should timeout since no incoming call
    assert!(wait_result.is_err());
}

#[tokio::test]
#[serial]
async fn test_accept_reject_incoming_via_coordinator() {
    let config = test_config();
    let coordinator = rvoip_sip::UnifiedCoordinator::new(config).await.unwrap();

    // Simulate accepting a call (would need real session ID)
    let fake_session_id = SessionId::new();
    let accept_result = coordinator.accept_call(&fake_session_id).await;
    // Will fail because session doesn't exist, but API works
    assert!(accept_result.is_err());

    // Simulate rejecting a call
    let reject_result = coordinator
        .reject(&fake_session_id)
        .with_status(486)
        .with_reason("Busy")
        .send()
        .await;
    // Will fail because session doesn't exist, but API works
    assert!(reject_result.is_err());
}

#[tokio::test]
#[serial]
async fn test_hangup_call() {
    let peer = StreamPeer::with_config(test_config()).await.unwrap();

    // Make a call
    let call_id = peer.invite("sip:bob@localhost:15118").send().await.unwrap();
    let handle = peer.coordinator().session(&call_id);

    // Hang up via SessionHandle
    let hangup_result = handle.hangup().await;
    assert!(hangup_result.is_ok());
}

// Integration test with two peers
#[tokio::test]
#[serial]
async fn test_peer_to_peer_call() {
    // Create two peers
    let alice = StreamPeer::with_config(test_config()).await.unwrap();
    let _bob = StreamPeer::with_config(test_config()).await.unwrap();

    // Alice calls Bob
    let call_id = alice
        .invite("sip:bob@localhost:15120")
        .send()
        .await
        .unwrap();
    let handle = alice.coordinator().session(&call_id);

    // Alice hangs up
    assert!(handle.hangup().await.is_ok());
}
