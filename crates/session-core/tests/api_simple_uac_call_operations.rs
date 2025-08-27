//! Call operation tests for the Simple UAC API

use rvoip_session_core::api::uac::{SimpleUacClient, SimpleCall};
use rvoip_session_core::api::uas::SimpleUasServer;
use rvoip_session_core::api::types::{CallState, AudioFrame};
use std::time::Duration;
use serial_test::serial;

#[tokio::test]
#[serial]
async fn test_hold_unhold() {
    println!("\n=== Testing Hold/Unhold Operations ===\n");
    
    let server = SimpleUasServer::always_accept("127.0.0.1:5110").await
        .expect("Failed to create server");
    
    let client = SimpleUacClient::new("alice")
        .port(5111)
        .await
        .expect("Failed to create client");
    
    let call = client.call("bob@127.0.0.1")
        .port(5110)
        .await
        .expect("Failed to initiate call");
    
    println!("✓ Call initiated: {}", call.id());
    
    // Check initial state
    assert_eq!(call.state().await, CallState::Active);
    println!("✓ Initial state: Active");
    
    // Put on hold
    call.hold().await.expect("Failed to put call on hold");
    assert_eq!(call.state().await, CallState::OnHold);
    println!("✓ Call put on hold");
    
    // Wait a bit
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    // Resume call
    call.unhold().await.expect("Failed to resume call");
    assert_eq!(call.state().await, CallState::Active);
    println!("✓ Call resumed");
    
    // Test multiple hold/unhold cycles
    for i in 1..=3 {
        call.hold().await.expect("Failed to hold");
        println!("✓ Hold cycle {}", i);
        tokio::time::sleep(Duration::from_millis(50)).await;
        
        call.unhold().await.expect("Failed to unhold");
        println!("✓ Unhold cycle {}", i);
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    
    call.hangup().await.expect("Failed to hang up");
    client.shutdown().await.expect("Failed to shutdown client");
    server.shutdown().await.expect("Failed to shutdown server");
}

#[tokio::test]
#[serial]
async fn test_mute_unmute() {
    println!("\n=== Testing Mute/Unmute Operations ===\n");
    
    let server = SimpleUasServer::always_accept("127.0.0.1:5112").await
        .expect("Failed to create server");
    
    let client = SimpleUacClient::new("alice")
        .port(5113)
        .await
        .expect("Failed to create client");
    
    let mut call = client.call("bob@127.0.0.1")
        .port(5112)
        .await
        .expect("Failed to initiate call");
    
    println!("✓ Call initiated: {}", call.id());
    
    // Get audio channels
    let (tx, _rx) = call.audio_channels().await;
    
    // Send audio while muting/unmuting
    let tx_clone = tx.clone();
    let audio_task = tokio::spawn(async move {
        for i in 0..20 {
            let frame = AudioFrame::new(vec![i as i16; 160], 8000, 1, 0);
            if tx_clone.send(frame).await.is_err() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(50)).await;
        }
    });
    
    // Test mute
    tokio::time::sleep(Duration::from_millis(100)).await;
    call.mute().await.expect("Failed to mute");
    println!("✓ Audio muted");
    
    // Audio should be muted now
    tokio::time::sleep(Duration::from_millis(200)).await;
    
    // Unmute
    call.unmute().await.expect("Failed to unmute");
    println!("✓ Audio unmuted");
    
    // Test multiple mute/unmute cycles
    for i in 1..=3 {
        tokio::time::sleep(Duration::from_millis(50)).await;
        call.mute().await.expect("Failed to mute");
        println!("✓ Mute cycle {}", i);
        
        tokio::time::sleep(Duration::from_millis(50)).await;
        call.unmute().await.expect("Failed to unmute");
        println!("✓ Unmute cycle {}", i);
    }
    
    audio_task.abort();
    call.hangup().await.expect("Failed to hang up");
    client.shutdown().await.expect("Failed to shutdown client");
    server.shutdown().await.expect("Failed to shutdown server");
}

#[tokio::test]
#[serial]
async fn test_dtmf_sending() {
    println!("\n=== Testing DTMF Sending ===\n");
    
    let server = SimpleUasServer::always_accept("127.0.0.1:5114").await
        .expect("Failed to create server");
    
    let client = SimpleUacClient::new("alice")
        .port(5115)
        .await
        .expect("Failed to create client");
    
    let call = client.call("bob@127.0.0.1")
        .port(5114)
        .await
        .expect("Failed to initiate call");
    
    println!("✓ Call initiated: {}", call.id());
    
    // Send single DTMF digit
    call.send_dtmf("1").await.expect("Failed to send DTMF 1");
    println!("✓ Sent DTMF: 1");
    
    // Send multiple DTMF digits
    call.send_dtmf("234").await.expect("Failed to send DTMF 234");
    println!("✓ Sent DTMF: 234");
    
    // Send DTMF with special characters
    call.send_dtmf("*#").await.expect("Failed to send DTMF *#");
    println!("✓ Sent DTMF: *#");
    
    // Send complete phone number
    call.send_dtmf("5551234").await.expect("Failed to send DTMF phone number");
    println!("✓ Sent DTMF: 5551234");
    
    // Send DTMF sequence with pauses
    for digit in "0123456789*#".chars() {
        call.send_dtmf(&digit.to_string()).await
            .expect(&format!("Failed to send DTMF {}", digit));
        println!("✓ Sent DTMF: {}", digit);
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    
    call.hangup().await.expect("Failed to hang up");
    client.shutdown().await.expect("Failed to shutdown client");
    server.shutdown().await.expect("Failed to shutdown server");
}

#[tokio::test]
#[serial]
async fn test_call_state_transitions() {
    println!("\n=== Testing Call State Transitions ===\n");
    
    let server = SimpleUasServer::always_accept("127.0.0.1:5116").await
        .expect("Failed to create server");
    
    let client = SimpleUacClient::new("alice")
        .port(5117)
        .await
        .expect("Failed to create client");
    
    let call = client.call("bob@127.0.0.1")
        .port(5116)
        .await
        .expect("Failed to initiate call");
    
    println!("✓ Call initiated: {}", call.id());
    
    // Check initial state
    let state = call.state().await;
    assert_eq!(state, CallState::Active);
    println!("✓ State: {:?}", state);
    
    // Hold -> OnHold
    call.hold().await.expect("Failed to hold");
    let state = call.state().await;
    assert_eq!(state, CallState::OnHold);
    println!("✓ After hold: {:?}", state);
    
    // Unhold -> Active
    call.unhold().await.expect("Failed to unhold");
    let state = call.state().await;
    assert_eq!(state, CallState::Active);
    println!("✓ After unhold: {:?}", state);
    
    // Hangup -> Terminated
    call.hangup().await.expect("Failed to hang up");
    // Can't check state after hangup as call is consumed
    println!("✓ Call terminated");
    
    client.shutdown().await.expect("Failed to shutdown client");
    server.shutdown().await.expect("Failed to shutdown server");
}

#[tokio::test]
#[serial]
async fn test_call_duration() {
    println!("\n=== Testing Call Duration Tracking ===\n");
    
    let server = SimpleUasServer::always_accept("127.0.0.1:5118").await
        .expect("Failed to create server");
    
    let client = SimpleUacClient::new("alice")
        .port(5119)
        .await
        .expect("Failed to create client");
    
    let call = client.call("bob@127.0.0.1")
        .port(5118)
        .await
        .expect("Failed to initiate call");
    
    println!("✓ Call initiated: {}", call.id());
    
    // Check initial duration (should be very small)
    let duration1 = call.duration();
    println!("✓ Initial duration: {:?}", duration1);
    assert!(duration1.as_millis() < 100);
    
    // Wait and check duration increases
    tokio::time::sleep(Duration::from_millis(500)).await;
    let duration2 = call.duration();
    println!("✓ After 500ms: {:?}", duration2);
    assert!(duration2.as_millis() >= 500);
    
    // Duration should keep increasing
    tokio::time::sleep(Duration::from_millis(500)).await;
    let duration3 = call.duration();
    println!("✓ After 1000ms: {:?}", duration3);
    assert!(duration3.as_millis() >= 1000);
    
    // Duration should work while on hold
    call.hold().await.expect("Failed to hold");
    tokio::time::sleep(Duration::from_millis(200)).await;
    let duration4 = call.duration();
    println!("✓ Duration while on hold: {:?}", duration4);
    assert!(duration4 > duration3);
    
    call.hangup().await.expect("Failed to hang up");
    client.shutdown().await.expect("Failed to shutdown client");
    server.shutdown().await.expect("Failed to shutdown server");
}

#[tokio::test]
#[serial]
async fn test_packet_loss_reporting() {
    println!("\n=== Testing Packet Loss Rate Reporting ===\n");
    
    let server = SimpleUasServer::always_accept("127.0.0.1:5120").await
        .expect("Failed to create server");
    
    let client = SimpleUacClient::new("alice")
        .port(5121)
        .await
        .expect("Failed to create client");
    
    let mut call = client.call("bob@127.0.0.1")
        .port(5120)
        .await
        .expect("Failed to initiate call");
    
    println!("✓ Call initiated: {}", call.id());
    
    // Get initial packet loss (should be 0 or very low)
    let loss1 = call.packet_loss_rate().await;
    println!("✓ Initial packet loss: {:.2}%", loss1 * 100.0);
    assert!(loss1 >= 0.0 && loss1 <= 1.0);
    
    // Send some audio
    let (tx, _rx) = call.audio_channels().await;
    for i in 0..10 {
        let frame = AudioFrame::new(vec![i as i16; 160], 8000, 1, 0);
        tx.send(frame).await.expect("Failed to send frame");
    }
    
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    // Check packet loss again
    let loss2 = call.packet_loss_rate().await;
    println!("✓ After sending audio: {:.2}%", loss2 * 100.0);
    assert!(loss2 >= 0.0 && loss2 <= 1.0);
    
    // Packet loss should work while on hold
    call.hold().await.expect("Failed to hold");
    let loss3 = call.packet_loss_rate().await;
    println!("✓ Packet loss while on hold: {:.2}%", loss3 * 100.0);
    assert!(loss3 >= 0.0 && loss3 <= 1.0);
    
    call.hangup().await.expect("Failed to hang up");
    client.shutdown().await.expect("Failed to shutdown client");
    server.shutdown().await.expect("Failed to shutdown server");
}

#[tokio::test]
#[serial]
async fn test_call_info_methods() {
    println!("\n=== Testing Call Information Methods ===\n");
    
    let server = SimpleUasServer::always_accept("127.0.0.1:5122").await
        .expect("Failed to create server");
    
    let client = SimpleUacClient::new("alice")
        .port(5123)
        .await
        .expect("Failed to create client");
    
    let call = client.call("bob@example.com")
        .port(5122)
        .call_id("test-call-info-123")
        .await
        .expect("Failed to initiate call");
    
    // Test call ID
    let id = call.id();
    println!("✓ Call ID: {}", id);
    assert!(!id.is_empty());
    
    // Test remote URI
    let remote = call.remote_uri();
    println!("✓ Remote URI: {}", remote);
    assert!(remote.contains("bob"));
    
    // Test state
    let state = call.state().await;
    println!("✓ Call state: {:?}", state);
    assert_eq!(state, CallState::Active);
    
    // Test duration
    tokio::time::sleep(Duration::from_millis(100)).await;
    let duration = call.duration();
    println!("✓ Call duration: {:?}", duration);
    assert!(duration.as_millis() >= 100);
    
    // Test packet loss
    let loss = call.packet_loss_rate().await;
    println!("✓ Packet loss: {:.2}%", loss * 100.0);
    assert!(loss >= 0.0 && loss <= 1.0);
    
    call.hangup().await.expect("Failed to hang up");
    client.shutdown().await.expect("Failed to shutdown client");
    server.shutdown().await.expect("Failed to shutdown server");
}