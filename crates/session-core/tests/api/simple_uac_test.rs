//! Test for the new Simple UAC API

use rvoip_session_core::api::uac::{SimpleUacClient, SimpleCall};
use rvoip_session_core::api::uas::{SimpleUasServer};
use std::time::Duration;
use serial_test::serial;

#[tokio::test]
#[serial]
async fn test_simple_uac_basic_call() {
    println!("\n=== Testing Simple UAC API ===\n");
    
    // Create a simple UAS server
    let server = SimpleUasServer::always_accept("127.0.0.1:19000").await
        .expect("Failed to create UAS server");
    
    println!("Server created on 127.0.0.1:19000");
    
    // Create a simple UAC client with builder pattern
    let client = SimpleUacClient::new("alice")
        .local_addr("127.0.0.1")
        .port(19001)
        .await
        .expect("Failed to create UAC client");
    
    println!("Client created: alice@127.0.0.1:19001");
    
    // Test registration (mock for now)
    client.register("sip:registrar@example.com").await
        .expect("Failed to register");
    
    println!("Client registered");
    
    // Make a call with default port
    println!("Making call to bob@127.0.0.1:19000...");
    let mut call = client.call("bob@127.0.0.1")
        .port(19000)
        .call_id("test-call-123")
        .await
        .expect("Failed to make call");
    
    println!("Call established with ID: {}", call.id());
    
    // Get audio channels
    let (audio_tx, mut audio_rx) = call.audio_channels();
    println!("Audio channels obtained");
    
    // Test sending audio
    tokio::spawn(async move {
        for i in 0..5 {
            let frame = rvoip_session_core::api::types::AudioFrame::new(
                vec![i as i16; 160],
                8000,
                1,
                (i * 160) as u32,
            );
            if audio_tx.send(frame).await.is_err() {
                break;
            }
            tokio::time::sleep(Duration::from_millis(20)).await;
        }
        println!("Finished sending test audio");
    });
    
    // Test receiving audio (with timeout)
    tokio::spawn(async move {
        let mut count = 0;
        while count < 5 {
            match tokio::time::timeout(Duration::from_millis(100), audio_rx.recv()).await {
                Ok(Some(_frame)) => {
                    count += 1;
                    println!("Received audio frame {}", count);
                }
                _ => break,
            }
        }
        println!("Received {} audio frames", count);
    });
    
    // Test call operations
    println!("\nTesting call operations:");
    
    // Get call info
    println!("Call ID: {}", call.id());
    println!("Remote URI: {}", call.remote_uri());
    println!("Duration: {:?}", call.duration());
    println!("State: {:?}", call.state().await);
    
    // Test DTMF
    call.send_dtmf("123#").await
        .expect("Failed to send DTMF");
    println!("Sent DTMF: 123#");
    
    // Test hold/unhold
    call.hold().await.expect("Failed to hold");
    println!("Call on hold");
    assert_eq!(call.state().await, rvoip_session_core::api::types::CallState::OnHold);
    
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    call.unhold().await.expect("Failed to unhold");
    println!("Call resumed");
    assert_eq!(call.state().await, rvoip_session_core::api::types::CallState::Active);
    
    // Test mute/unmute
    call.mute().await.expect("Failed to mute");
    println!("Audio muted");
    
    call.unmute().await.expect("Failed to unmute");
    println!("Audio unmuted");
    
    // Get quality metrics
    let packet_loss = call.packet_loss_rate().await;
    println!("Packet loss rate: {:.2}%", packet_loss * 100.0);
    
    // Hang up
    println!("\nHanging up call...");
    call.hangup().await.expect("Failed to hang up");
    
    // Unregister
    client.unregister().await.expect("Failed to unregister");
    println!("Client unregistered");
    
    // Shutdown
    client.shutdown().await.expect("Failed to shutdown client");
    server.shutdown().await.expect("Failed to shutdown server");
    
    println!("\n=== Test Complete ===\n");
}

#[tokio::test]
#[serial]
async fn test_simple_uac_protocol_detection() {
    println!("\n=== Testing Protocol Auto-Detection ===\n");
    
    // Create client
    let client = SimpleUacClient::new("alice")
        .port(19002)
        .await
        .expect("Failed to create client");
    
    // These would make calls if we had a server running
    // Just testing that the API accepts different formats
    
    // Test SIP protocol detection
    let _ = client.call("bob@example.com"); // Should detect as SIP
    let _ = client.call("sip:bob@example.com"); // Explicit SIP
    
    // Test TEL protocol detection  
    let _ = client.call("+14155551234"); // Should detect as TEL
    let _ = client.call("tel:+14155551234"); // Explicit TEL
    let _ = client.call("911"); // Emergency number
    
    client.shutdown().await.expect("Failed to shutdown");
    
    println!("Protocol detection test complete\n");
}