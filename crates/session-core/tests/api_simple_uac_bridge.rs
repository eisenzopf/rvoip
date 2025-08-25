//! Bridge (3-way conference) tests for the Simple UAC API

use rvoip_session_core::api::uac::{SimpleUacClient, SimpleCall};
use rvoip_session_core::api::uas::SimpleUasServer;
use rvoip_session_core::api::types::AudioFrame;
use std::time::Duration;
use serial_test::serial;

#[tokio::test]
#[serial]
#[ignore = "Bridge operation not yet implemented"]
async fn test_basic_bridge() {
    println!("\n=== Testing Basic Call Bridging ===\n");
    
    // Create two UAS servers for Bob and Charlie
    let bob_server = SimpleUasServer::always_accept("127.0.0.1:5150").await
        .expect("Failed to create Bob's server");
    
    let charlie_server = SimpleUasServer::always_accept("127.0.0.1:5151").await
        .expect("Failed to create Charlie's server");
    
    // Create UAC client for Alice
    let alice_client = SimpleUacClient::new("alice")
        .port(5152)
        .await
        .expect("Failed to create Alice's client");
    
    // Alice calls Bob
    let call_to_bob = alice_client.call("bob@127.0.0.1")
        .port(5150)
        .call_id("alice-to-bob")
        .await
        .expect("Failed to call Bob");
    
    println!("✓ Alice called Bob: {}", call_to_bob.id());
    
    // Alice calls Charlie
    let call_to_charlie = alice_client.call("charlie@127.0.0.1")
        .port(5151)
        .call_id("alice-to-charlie")
        .await
        .expect("Failed to call Charlie");
    
    println!("✓ Alice called Charlie: {}", call_to_charlie.id());
    
    // Bridge the two calls together
    println!("Bridging calls...");
    call_to_bob.bridge(call_to_charlie).await
        .expect("Failed to bridge calls");
    
    println!("✓ Successfully bridged Bob and Charlie through Alice");
    println!("  Note: Bridge operation is placeholder - full implementation pending");
    
    // The bridge consumes call_to_charlie, call_to_bob continues
    call_to_bob.hangup().await.expect("Failed to hang up");
    
    alice_client.shutdown().await.expect("Failed to shutdown Alice");
    bob_server.shutdown().await.expect("Failed to shutdown Bob");
    charlie_server.shutdown().await.expect("Failed to shutdown Charlie");
}

#[tokio::test]
#[serial]
#[ignore = "Bridge operation not yet implemented"]
async fn test_bridge_with_audio() {
    println!("\n=== Testing Bridge with Audio Flow ===\n");
    
    let bob_server = SimpleUasServer::always_accept("127.0.0.1:5153").await
        .expect("Failed to create Bob's server");
    
    let charlie_server = SimpleUasServer::always_accept("127.0.0.1:5154").await
        .expect("Failed to create Charlie's server");
    
    let alice_client = SimpleUacClient::new("alice")
        .port(5155)
        .await
        .expect("Failed to create Alice's client");
    
    // Alice calls Bob
    let mut call_to_bob = alice_client.call("bob@127.0.0.1")
        .port(5153)
        .await
        .expect("Failed to call Bob");
    
    println!("✓ Alice called Bob: {}", call_to_bob.id());
    
    // Get audio channels for Bob's call
    let (bob_tx, _bob_rx) = call_to_bob.audio_channels();
    
    // Send some audio to Bob
    for i in 0..5 {
        let frame = AudioFrame::new(vec![i as i16; 160], 8000, 1, 0);
        bob_tx.send(frame).await.expect("Failed to send audio to Bob");
    }
    println!("✓ Sent audio to Bob");
    
    // Alice calls Charlie
    let mut call_to_charlie = alice_client.call("charlie@127.0.0.1")
        .port(5154)
        .await
        .expect("Failed to call Charlie");
    
    println!("✓ Alice called Charlie: {}", call_to_charlie.id());
    
    // Get audio channels for Charlie's call
    let (charlie_tx, _charlie_rx) = call_to_charlie.audio_channels();
    
    // Send some audio to Charlie
    for i in 0..5 {
        let frame = AudioFrame::new(vec![(100 + i) as i16; 160], 8000, 1, 0);
        charlie_tx.send(frame).await.expect("Failed to send audio to Charlie");
    }
    println!("✓ Sent audio to Charlie");
    
    // Bridge the calls
    call_to_bob.bridge(call_to_charlie).await
        .expect("Failed to bridge calls");
    
    println!("✓ Calls bridged with audio channels active");
    
    // Continue sending audio after bridge
    for i in 0..5 {
        let frame = AudioFrame::new(vec![(200 + i) as i16; 160], 8000, 1, 0);
        bob_tx.send(frame).await.expect("Failed to send audio after bridge");
    }
    println!("✓ Continued sending audio after bridge");
    
    call_to_bob.hangup().await.expect("Failed to hang up");
    
    alice_client.shutdown().await.expect("Failed to shutdown Alice");
    bob_server.shutdown().await.expect("Failed to shutdown Bob");
    charlie_server.shutdown().await.expect("Failed to shutdown Charlie");
}

#[tokio::test]
#[serial]
#[ignore = "Bridge operation not yet implemented"]
async fn test_bridge_with_hold() {
    println!("\n=== Testing Bridge with One Call on Hold ===\n");
    
    let bob_server = SimpleUasServer::always_accept("127.0.0.1:5156").await
        .expect("Failed to create Bob's server");
    
    let charlie_server = SimpleUasServer::always_accept("127.0.0.1:5157").await
        .expect("Failed to create Charlie's server");
    
    let alice_client = SimpleUacClient::new("alice")
        .port(5158)
        .await
        .expect("Failed to create Alice's client");
    
    // Alice calls Bob
    let call_to_bob = alice_client.call("bob@127.0.0.1")
        .port(5156)
        .await
        .expect("Failed to call Bob");
    
    println!("✓ Alice called Bob: {}", call_to_bob.id());
    
    // Put Bob on hold
    call_to_bob.hold().await.expect("Failed to put Bob on hold");
    println!("✓ Bob is on hold");
    
    // Alice calls Charlie while Bob is on hold
    let call_to_charlie = alice_client.call("charlie@127.0.0.1")
        .port(5157)
        .await
        .expect("Failed to call Charlie");
    
    println!("✓ Alice called Charlie while Bob on hold: {}", call_to_charlie.id());
    
    // Bridge the calls (Bob on hold + Charlie active)
    println!("Bridging calls (Bob on hold)...");
    call_to_bob.bridge(call_to_charlie).await
        .expect("Failed to bridge calls");
    
    println!("✓ Successfully bridged calls with Bob initially on hold");
    
    // Resume Bob (if bridge doesn't auto-resume)
    call_to_bob.unhold().await.expect("Failed to resume Bob");
    println!("✓ Bob resumed after bridge");
    
    call_to_bob.hangup().await.expect("Failed to hang up");
    
    alice_client.shutdown().await.expect("Failed to shutdown Alice");
    bob_server.shutdown().await.expect("Failed to shutdown Bob");
    charlie_server.shutdown().await.expect("Failed to shutdown Charlie");
}

#[tokio::test]
#[serial]
#[ignore = "Bridge operation not yet implemented"]
async fn test_multiple_bridges() {
    println!("\n=== Testing Multiple Bridge Operations ===\n");
    
    // Create multiple servers
    let server1 = SimpleUasServer::always_accept("127.0.0.1:5159").await
        .expect("Failed to create server 1");
    
    let server2 = SimpleUasServer::always_accept("127.0.0.1:5160").await
        .expect("Failed to create server 2");
    
    let server3 = SimpleUasServer::always_accept("127.0.0.1:5161").await
        .expect("Failed to create server 3");
    
    let server4 = SimpleUasServer::always_accept("127.0.0.1:5162").await
        .expect("Failed to create server 4");
    
    let client = SimpleUacClient::new("alice")
        .port(5163)
        .await
        .expect("Failed to create client");
    
    // Create first pair of calls
    println!("\n--- First Bridge ---");
    let call1 = client.call("party1@127.0.0.1")
        .port(5159)
        .await
        .expect("Failed to call party 1");
    println!("✓ Called party 1: {}", call1.id());
    
    let call2 = client.call("party2@127.0.0.1")
        .port(5160)
        .await
        .expect("Failed to call party 2");
    println!("✓ Called party 2: {}", call2.id());
    
    call1.bridge(call2).await.expect("Failed to bridge first pair");
    println!("✓ Bridged parties 1 and 2");
    
    // Create second pair of calls
    println!("\n--- Second Bridge ---");
    let call3 = client.call("party3@127.0.0.1")
        .port(5161)
        .await
        .expect("Failed to call party 3");
    println!("✓ Called party 3: {}", call3.id());
    
    let call4 = client.call("party4@127.0.0.1")
        .port(5162)
        .await
        .expect("Failed to call party 4");
    println!("✓ Called party 4: {}", call4.id());
    
    call3.bridge(call4).await.expect("Failed to bridge second pair");
    println!("✓ Bridged parties 3 and 4");
    
    // Clean up first bridge
    call1.hangup().await.expect("Failed to hang up call 1");
    println!("✓ First bridge terminated");
    
    // Clean up second bridge
    call3.hangup().await.expect("Failed to hang up call 3");
    println!("✓ Second bridge terminated");
    
    client.shutdown().await.expect("Failed to shutdown client");
    server1.shutdown().await.expect("Failed to shutdown server 1");
    server2.shutdown().await.expect("Failed to shutdown server 2");
    server3.shutdown().await.expect("Failed to shutdown server 3");
    server4.shutdown().await.expect("Failed to shutdown server 4");
}

#[tokio::test]
#[serial]
#[ignore = "Bridge operation not yet implemented"]
async fn test_bridge_timing() {
    println!("\n=== Testing Bridge Timing and Duration ===\n");
    
    let bob_server = SimpleUasServer::always_accept("127.0.0.1:5164").await
        .expect("Failed to create Bob's server");
    
    let charlie_server = SimpleUasServer::always_accept("127.0.0.1:5165").await
        .expect("Failed to create Charlie's server");
    
    let alice_client = SimpleUacClient::new("alice")
        .port(5166)
        .await
        .expect("Failed to create Alice's client");
    
    // Alice calls Bob
    let call_to_bob = alice_client.call("bob@127.0.0.1")
        .port(5164)
        .await
        .expect("Failed to call Bob");
    
    let bob_start = call_to_bob.duration();
    println!("✓ Bob call started, initial duration: {:?}", bob_start);
    
    // Wait a bit
    tokio::time::sleep(Duration::from_millis(500)).await;
    
    // Alice calls Charlie
    let call_to_charlie = alice_client.call("charlie@127.0.0.1")
        .port(5165)
        .await
        .expect("Failed to call Charlie");
    
    let charlie_start = call_to_charlie.duration();
    println!("✓ Charlie call started, initial duration: {:?}", charlie_start);
    
    // Check Bob's duration before bridge
    let bob_before_bridge = call_to_bob.duration();
    println!("✓ Bob's duration before bridge: {:?}", bob_before_bridge);
    assert!(bob_before_bridge >= Duration::from_millis(500));
    
    // Bridge the calls
    call_to_bob.bridge(call_to_charlie).await
        .expect("Failed to bridge calls");
    println!("✓ Calls bridged");
    
    // Wait and check duration after bridge
    tokio::time::sleep(Duration::from_millis(300)).await;
    
    let bob_after_bridge = call_to_bob.duration();
    println!("✓ Bob's duration after bridge: {:?}", bob_after_bridge);
    assert!(bob_after_bridge >= Duration::from_millis(800));
    
    call_to_bob.hangup().await.expect("Failed to hang up");
    
    alice_client.shutdown().await.expect("Failed to shutdown Alice");
    bob_server.shutdown().await.expect("Failed to shutdown Bob");
    charlie_server.shutdown().await.expect("Failed to shutdown Charlie");
}