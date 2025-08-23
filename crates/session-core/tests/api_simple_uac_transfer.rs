//! Transfer operation tests for the Simple UAC API

use rvoip_session_core::api::uac::{SimpleUacClient, SimpleCall};
use rvoip_session_core::api::uas::SimpleUasServer;
use std::time::Duration;
use serial_test::serial;

#[tokio::test]
#[serial] 
async fn test_blind_transfer() {
    println!("\n=== Testing Blind Transfer ===\n");
    
    // Create three parties: Alice (caller), Bob (initial recipient), Charlie (transfer target)
    let bob_server = SimpleUasServer::always_accept("127.0.0.1:5130").await
        .expect("Failed to create Bob's server");
    
    let charlie_server = SimpleUasServer::always_accept("127.0.0.1:5131").await
        .expect("Failed to create Charlie's server");
    
    let alice_client = SimpleUacClient::new("alice")
        .port(5132)
        .await
        .expect("Failed to create Alice's client");
    
    // Alice calls Bob
    let call = alice_client.call("bob@127.0.0.1")
        .port(5130)
        .await
        .expect("Failed to initiate call to Bob");
    
    println!("✓ Alice called Bob: {}", call.id());
    
    // Let the call establish
    tokio::time::sleep(Duration::from_millis(200)).await;
    
    // Alice transfers the call to Charlie (blind transfer)
    println!("Initiating blind transfer to Charlie...");
    call.transfer("charlie@127.0.0.1:5131").await
        .expect("Failed to transfer call to Charlie");
    
    println!("✓ Call transferred from Bob to Charlie");
    
    // Note: After transfer, the original call object is consumed
    // In a real scenario, Bob would receive the REFER and initiate a new call to Charlie
    
    alice_client.shutdown().await.expect("Failed to shutdown Alice");
    bob_server.shutdown().await.expect("Failed to shutdown Bob");
    charlie_server.shutdown().await.expect("Failed to shutdown Charlie");
}

#[tokio::test]
#[serial]
async fn test_attended_transfer_placeholder() {
    println!("\n=== Testing Attended Transfer (Placeholder) ===\n");
    
    // Create three parties
    let bob_server = SimpleUasServer::always_accept("127.0.0.1:5133").await
        .expect("Failed to create Bob's server");
    
    let charlie_server = SimpleUasServer::always_accept("127.0.0.1:5134").await
        .expect("Failed to create Charlie's server");
    
    let alice_client = SimpleUacClient::new("alice")
        .port(5135)
        .await
        .expect("Failed to create Alice's client");
    
    // Alice calls Bob
    let call_to_bob = alice_client.call("bob@127.0.0.1")
        .port(5133)
        .await
        .expect("Failed to call Bob");
    
    println!("✓ Alice called Bob: {}", call_to_bob.id());
    
    // Alice calls Charlie (consultation call)
    let call_to_charlie = alice_client.call("charlie@127.0.0.1")
        .port(5134)
        .await
        .expect("Failed to call Charlie");
    
    println!("✓ Alice called Charlie: {}", call_to_charlie.id());
    
    // Alice performs attended transfer (transfers Bob to Charlie)
    println!("Initiating attended transfer...");
    call_to_bob.transfer("charlie@127.0.0.1:5134")
        .attended(call_to_charlie)
        .await
        .expect("Failed to perform attended transfer");
    
    println!("✓ Attended transfer completed (placeholder implementation)");
    
    alice_client.shutdown().await.expect("Failed to shutdown Alice");
    bob_server.shutdown().await.expect("Failed to shutdown Bob");
    charlie_server.shutdown().await.expect("Failed to shutdown Charlie");
}

#[tokio::test]
#[serial]
async fn test_transfer_with_custom_uri_formats() {
    println!("\n=== Testing Transfer with Various URI Formats ===\n");
    
    let server = SimpleUasServer::always_accept("127.0.0.1:5136").await
        .expect("Failed to create server");
    
    let client = SimpleUacClient::new("alice")
        .port(5137)
        .await
        .expect("Failed to create client");
    
    // Test transfer with different URI formats
    let test_cases = vec![
        ("bob@example.com", "SIP URI without protocol"),
        ("sip:bob@example.com:5060", "Full SIP URI"),
        ("tel:+14155551234", "TEL URI"),
        ("+14155551234", "Phone number"),
    ];
    
    for (uri, description) in test_cases {
        let call = client.call("test@127.0.0.1")
            .port(5136)
            .await
            .expect("Failed to initiate call");
        
        println!("Testing transfer to {}: {}", description, uri);
        
        call.transfer(uri).await
            .expect(&format!("Failed to transfer to {}", uri));
        
        println!("✓ Successfully initiated transfer to {}", uri);
        
        // Brief pause between tests
        tokio::time::sleep(Duration::from_millis(100)).await;
    }
    
    client.shutdown().await.expect("Failed to shutdown client");
    server.shutdown().await.expect("Failed to shutdown server");
}

#[tokio::test]
#[serial]
async fn test_transfer_during_hold() {
    println!("\n=== Testing Transfer While Call is on Hold ===\n");
    
    let bob_server = SimpleUasServer::always_accept("127.0.0.1:5138").await
        .expect("Failed to create Bob's server");
    
    let charlie_server = SimpleUasServer::always_accept("127.0.0.1:5139").await
        .expect("Failed to create Charlie's server");
    
    let alice_client = SimpleUacClient::new("alice")
        .port(5140)
        .await
        .expect("Failed to create Alice's client");
    
    // Alice calls Bob
    let call = alice_client.call("bob@127.0.0.1")
        .port(5138)
        .await
        .expect("Failed to call Bob");
    
    println!("✓ Alice called Bob: {}", call.id());
    
    // Put Bob on hold
    call.hold().await.expect("Failed to put Bob on hold");
    println!("✓ Bob is on hold");
    
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    // Transfer while on hold
    println!("Transferring call while on hold...");
    call.transfer("charlie@127.0.0.1:5139").await
        .expect("Failed to transfer call while on hold");
    
    println!("✓ Successfully transferred call that was on hold");
    
    alice_client.shutdown().await.expect("Failed to shutdown Alice");
    bob_server.shutdown().await.expect("Failed to shutdown Bob");
    charlie_server.shutdown().await.expect("Failed to shutdown Charlie");
}

#[tokio::test]
#[serial]
async fn test_multiple_transfers_sequence() {
    println!("\n=== Testing Multiple Sequential Transfers ===\n");
    
    let server1 = SimpleUasServer::always_accept("127.0.0.1:5141").await
        .expect("Failed to create server 1");
    
    let server2 = SimpleUasServer::always_accept("127.0.0.1:5142").await
        .expect("Failed to create server 2");
    
    let client = SimpleUacClient::new("alice")
        .port(5143)
        .await
        .expect("Failed to create client");
    
    // Make multiple calls and transfer each
    for i in 1..=3 {
        println!("\n--- Transfer {} ---", i);
        
        let call = client.call("initial@127.0.0.1")
            .port(5141)
            .call_id(&format!("transfer-test-{}", i))
            .await
            .expect(&format!("Failed to make call {}", i));
        
        println!("✓ Call {} initiated: {}", i, call.id());
        
        tokio::time::sleep(Duration::from_millis(100)).await;
        
        // Transfer to second server
        call.transfer(&format!("target{}@127.0.0.1:5142", i)).await
            .expect(&format!("Failed to transfer call {}", i));
        
        println!("✓ Call {} transferred", i);
        
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    
    client.shutdown().await.expect("Failed to shutdown client");
    server1.shutdown().await.expect("Failed to shutdown server 1");
    server2.shutdown().await.expect("Failed to shutdown server 2");
}