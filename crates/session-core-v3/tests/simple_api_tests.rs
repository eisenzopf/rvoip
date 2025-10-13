//! Tests for the simple API
//!
//! These tests demonstrate the simple peer API usage

use rvoip_session_core_v2::api::simple::{SimplePeer, CallId};
use rvoip_session_core_v2::api::unified::Config;
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
    }
}

#[tokio::test]
#[serial]
async fn test_create_peer() {
    let peer = SimplePeer::new("alice").await;
    assert!(peer.is_ok());
}

#[tokio::test]
#[serial]
async fn test_make_outgoing_call() {
    let peer = SimplePeer::with_config("alice", test_config(15100)).await.unwrap();
    
    // Make a call
    let call_id = peer.call("sip:bob@localhost:15101").await;
    assert!(call_id.is_ok());
    
    // The call should be in Initiating state
    // In a real test, we'd check the state
}

#[tokio::test]
#[serial]
async fn test_hold_resume_call() {
    let peer = SimplePeer::with_config("alice", test_config(15102)).await.unwrap();
    
    // Make a call
    let call_id = peer.call("sip:bob@localhost:15103").await.unwrap();
    
    // Put on hold
    let hold_result = peer.hold(&call_id).await;
    assert!(hold_result.is_ok());
    
    // Resume
    let resume_result = peer.resume(&call_id).await;
    assert!(resume_result.is_ok());
}

#[tokio::test]
#[serial]
async fn test_send_dtmf() {
    let peer = SimplePeer::with_config("alice", test_config(15104)).await.unwrap();
    
    // Make a call
    let call_id = peer.call("sip:bob@localhost:15105").await.unwrap();
    
    // Send DTMF digits
    assert!(peer.send_dtmf(&call_id, '1').await.is_ok());
    assert!(peer.send_dtmf(&call_id, '2').await.is_ok());
    assert!(peer.send_dtmf(&call_id, '3').await.is_ok());
    assert!(peer.send_dtmf(&call_id, '#').await.is_ok());
}

#[tokio::test]
#[serial]
async fn test_transfer_call() {
    let peer = SimplePeer::with_config("alice", test_config(15106)).await.unwrap();
    
    // Make a call
    let call_id = peer.call("sip:bob@localhost:15107").await.unwrap();
    
    // Transfer to another party
    let transfer_result = peer.transfer(&call_id, "sip:charlie@localhost:15108").await;
    assert!(transfer_result.is_ok());
}

#[tokio::test]
#[serial]
async fn test_recording() {
    let peer = SimplePeer::with_config("alice", test_config(15109)).await.unwrap();
    
    // Make a call
    let call_id = peer.call("sip:bob@localhost:15110").await.unwrap();
    
    // Start recording
    assert!(peer.start_recording(&call_id).await.is_ok());
    
    // Stop recording
    assert!(peer.stop_recording(&call_id).await.is_ok());
}

#[tokio::test]
#[serial]
async fn test_conference_creation() {
    let peer = SimplePeer::with_config("alice", test_config(15111)).await.unwrap();
    
    // Make first call
    let call1 = peer.call("sip:bob@localhost:15112").await.unwrap();
    
    // Create conference from the call
    let conf_result = peer.create_conference(&call1, "Test Conference").await;
    assert!(conf_result.is_ok());
    
    // Make second call
    let call2 = peer.call("sip:charlie@localhost:15113").await.unwrap();
    
    // Add second call to conference
    let add_result = peer.add_to_conference(&call1, &call2).await;
    assert!(add_result.is_ok());
}

#[tokio::test]
#[serial]
async fn test_incoming_call_non_blocking() {
    let mut peer = SimplePeer::with_config("alice", test_config(15114)).await.unwrap();
    
    // Check for incoming call (should be none)
    let incoming = peer.incoming_call().await;
    assert!(incoming.is_none());
}

#[tokio::test]
#[serial]
async fn test_incoming_call_blocking() {
    let mut peer = SimplePeer::with_config("alice", test_config(15115)).await.unwrap();
    
    // Wait for incoming call with timeout
    let wait_result = timeout(
        Duration::from_millis(100),
        peer.wait_for_call()
    ).await;
    
    // Should timeout since no incoming call
    assert!(wait_result.is_err());
}

#[tokio::test]
#[serial]
async fn test_accept_reject_incoming() {
    let peer = SimplePeer::with_config("alice", test_config(15116)).await.unwrap();
    
    // In a real test, we'd have another peer make a call to us
    // For now, we'll test the API structure
    
    // Simulate accepting a call (would need real session ID)
    let fake_call_id = CallId::from(rvoip_session_core_v2::SessionId::new());
    let accept_result = peer.accept(&fake_call_id).await;
    // Will fail because session doesn't exist, but API works
    assert!(accept_result.is_err());
    
    // Simulate rejecting a call
    let reject_result = peer.reject(&fake_call_id).await;
    // Will fail because session doesn't exist, but API works
    assert!(reject_result.is_err());
}

#[tokio::test]
#[serial]
async fn test_hangup_call() {
    let peer = SimplePeer::with_config("alice", test_config(15117)).await.unwrap();
    
    // Make a call
    let call_id = peer.call("sip:bob@localhost:15118").await.unwrap();
    
    // Hang up
    let hangup_result = peer.hangup(&call_id).await;
    assert!(hangup_result.is_ok());
}

// Integration test with two peers
#[tokio::test]
#[serial]
async fn test_peer_to_peer_call() {
    // Create two peers
    let alice = SimplePeer::with_config("alice", test_config(15119)).await.unwrap();
    let _bob = SimplePeer::with_config("bob", test_config(15120)).await.unwrap();
    
    // Alice calls Bob
    let call_id = alice.call("sip:bob@localhost:15120").await.unwrap();
    
    // In a real implementation with proper signaling, Bob would receive the call
    // For now, we just test the APIs exist and work
    
    // Alice hangs up
    assert!(alice.hangup(&call_id).await.is_ok());
}
