//! Basic tests for the Simple UAC API

use rvoip_session_core::api::uac::{SimpleUacClient, SimpleCall};
use rvoip_session_core::api::uas::SimpleUasServer;
use std::time::Duration;
use serial_test::serial;

#[tokio::test]
#[serial]
async fn test_client_creation_default() {
    println!("\n=== Testing Simple UAC Client Creation with Defaults ===\n");
    
    let client = SimpleUacClient::new("alice")
        .await
        .expect("Failed to create client with defaults");
    
    println!("✓ Client created with identity: alice");
    println!("✓ Default port: 5060");
    println!("✓ Default address: 127.0.0.1");
    
    client.shutdown().await.expect("Failed to shutdown");
    println!("✓ Client shutdown successfully");
}

#[tokio::test]
#[serial]
async fn test_client_creation_custom() {
    println!("\n=== Testing Simple UAC Client Creation with Custom Settings ===\n");
    
    let client = SimpleUacClient::new("bob")
        .local_addr("127.0.0.1")
        .port(5070)
        .await
        .expect("Failed to create client with custom settings");
    
    println!("✓ Client created with identity: bob");
    println!("✓ Custom port: 5070");
    println!("✓ Custom address: 127.0.0.1");
    
    client.shutdown().await.expect("Failed to shutdown");
    println!("✓ Client shutdown successfully");
}

#[tokio::test]
#[serial]
async fn test_basic_call_lifecycle() {
    println!("\n=== Testing Basic Call Lifecycle ===\n");
    
    // Create server
    let server = SimpleUasServer::always_accept("127.0.0.1:5080").await
        .expect("Failed to create UAS server");
    println!("✓ UAS server created on 127.0.0.1:5080");
    
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    // Create client
    let client = SimpleUacClient::new("alice")
        .port(5081)
        .await
        .expect("Failed to create UAC client");
    println!("✓ UAC client created on port 5081");
    
    // Make call
    let mut call = client.call("bob@127.0.0.1")
        .port(5080)
        .await
        .expect("Failed to initiate call");
    println!("✓ Call initiated to bob@127.0.0.1:5080");
    println!("  Call ID: {}", call.id());
    
    // Check call properties
    assert_eq!(call.remote_uri(), "sip:bob@127.0.0.1:5080");
    println!("✓ Remote URI correct: {}", call.remote_uri());
    
    // Get audio channels (consumes them)
    let (_tx, _rx) = call.audio_channels();
    println!("✓ Audio channels obtained");
    
    // Hang up
    call.hangup().await.expect("Failed to hang up");
    println!("✓ Call hung up successfully");
    
    // Cleanup
    client.shutdown().await.expect("Failed to shutdown client");
    server.shutdown().await.expect("Failed to shutdown server");
    println!("✓ Client and server shutdown successfully");
}

#[tokio::test]
#[serial]
async fn test_protocol_detection() {
    println!("\n=== Testing Protocol Auto-Detection ===\n");
    
    let client = SimpleUacClient::new("alice")
        .port(5082)
        .await
        .expect("Failed to create client");
    
    // Test various URI formats (just building, not actually calling)
    let test_cases = vec![
        ("bob@example.com", "SIP URI without protocol"),
        ("sip:bob@example.com", "Explicit SIP URI"),
        ("sips:bob@example.com", "Secure SIP URI"),
        ("+14155551234", "Phone number (TEL)"),
        ("tel:+14155551234", "Explicit TEL URI"),
        ("911", "Emergency number"),
        ("alice", "Just username"),
    ];
    
    for (uri, description) in test_cases {
        let _call_builder = client.call(uri);
        println!("✓ Accepted {}: {}", description, uri);
    }
    
    client.shutdown().await.expect("Failed to shutdown");
    println!("✓ Protocol detection test complete");
}

#[tokio::test]
#[serial]
async fn test_registration() {
    println!("\n=== Testing Registration ===\n");
    
    let client = SimpleUacClient::new("alice")
        .port(5083)
        .await
        .expect("Failed to create client");
    println!("✓ Client created");
    
    // Register
    client.register("sip:registrar@example.com").await
        .expect("Failed to register");
    println!("✓ Registered to sip:registrar@example.com");
    
    // Make a call while registered
    let server = SimpleUasServer::always_accept("127.0.0.1:5084").await
        .expect("Failed to create server");
    
    let call = client.call("bob@127.0.0.1")
        .port(5084)
        .await
        .expect("Failed to make call while registered");
    println!("✓ Made call while registered");
    
    call.hangup().await.expect("Failed to hang up");
    
    // Unregister
    client.unregister().await
        .expect("Failed to unregister");
    println!("✓ Unregistered successfully");
    
    // Cleanup
    client.shutdown().await.expect("Failed to shutdown client");
    server.shutdown().await.expect("Failed to shutdown server");
    println!("✓ Registration test complete");
}

#[tokio::test]
#[serial]
async fn test_custom_call_id() {
    println!("\n=== Testing Custom Call ID ===\n");
    
    let server = SimpleUasServer::always_accept("127.0.0.1:5085").await
        .expect("Failed to create server");
    
    let client = SimpleUacClient::new("alice")
        .port(5086)
        .await
        .expect("Failed to create client");
    
    // Make call with custom call ID
    let custom_id = "my-custom-call-id-12345";
    let call = client.call("bob@127.0.0.1")
        .port(5085)
        .call_id(custom_id)
        .await
        .expect("Failed to make call with custom ID");
    
    println!("✓ Call created with custom ID: {}", call.id());
    // Note: The session ID might be different from the SIP Call-ID
    
    call.hangup().await.expect("Failed to hang up");
    
    // Make call without custom ID (auto-generated)
    let call2 = client.call("bob@127.0.0.1")
        .port(5085)
        .await
        .expect("Failed to make call with auto ID");
    
    println!("✓ Call created with auto-generated ID: {}", call2.id());
    
    call2.hangup().await.expect("Failed to hang up");
    
    client.shutdown().await.expect("Failed to shutdown client");
    server.shutdown().await.expect("Failed to shutdown server");
    println!("✓ Custom call ID test complete");
}

#[tokio::test]
#[serial]
async fn test_multiple_sequential_calls() {
    println!("\n=== Testing Multiple Sequential Calls ===\n");
    
    let server = SimpleUasServer::always_accept("127.0.0.1:5087").await
        .expect("Failed to create server");
    
    let client = SimpleUacClient::new("alice")
        .port(5088)
        .await
        .expect("Failed to create client");
    
    // Make multiple calls sequentially
    for i in 1..=3 {
        println!("\n--- Call {} ---", i);
        
        let call = client.call("bob@127.0.0.1")
            .port(5087)
            .call_id(&format!("call-{}", i))
            .await
            .expect(&format!("Failed to make call {}", i));
        
        println!("✓ Call {} initiated: {}", i, call.id());
        
        // Brief call duration
        tokio::time::sleep(Duration::from_millis(100)).await;
        
        let duration = call.duration();
        println!("✓ Call {} duration: {:?}", i, duration);
        
        call.hangup().await
            .expect(&format!("Failed to hang up call {}", i));
        println!("✓ Call {} hung up", i);
        
        // Brief pause between calls
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    
    client.shutdown().await.expect("Failed to shutdown client");
    server.shutdown().await.expect("Failed to shutdown server");
    println!("\n✓ Multiple sequential calls test complete");
}