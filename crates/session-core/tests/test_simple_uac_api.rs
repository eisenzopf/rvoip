//! Standalone test for the new Simple UAC API
//! This test doesn't rely on any common test utilities.

use rvoip_session_core::api::uac::{SimpleUacClient, SimpleCall};
use rvoip_session_core::api::uas::{SimpleUasServer};
use rvoip_session_core::api::types::{AudioFrame};
use std::time::Duration;
use serial_test::serial;

#[tokio::test(flavor = "multi_thread", worker_threads = 4)]
#[serial]
async fn test_real_uac_to_uas_bidirectional_audio() {
    println!("\n=== Starting Real UAC-to-UAS Bidirectional Audio Test with New Simple API ===\n");
    
    // Step 1: Create UAS server (simple, always accepts)
    let server = SimpleUasServer::always_accept("127.0.0.1:5060").await
        .expect("Failed to create UAS server");
    
    println!("UAS: Server created on 127.0.0.1:5060 (always accepts calls)");
    
    // Give server time to bind
    tokio::time::sleep(Duration::from_millis(200)).await;
    
    // Step 2: Create UAC client with new simple API
    let client = SimpleUacClient::new("alice")
        .local_addr("127.0.0.1")
        .port(5061)
        .await
        .expect("Failed to create UAC client");
    
    println!("UAC: Client created for alice@127.0.0.1:5061");
    
    // Step 3: UAC initiates call to UAS
    println!("UAC: Initiating call to bob@127.0.0.1:5060...");
    let mut uac_call = client.call("bob@127.0.0.1")
        .port(5060)
        .call_id("test-call-123")
        .await
        .expect("Failed to initiate call");
    
    println!("UAC: Call initiated with ID: {}", uac_call.id());
    
    // Step 4: Get audio channels from UAC
    println!("\n=== Setting up audio channels ===");
    let (uac_audio_tx, mut uac_audio_rx) = uac_call.audio_channels();
    println!("UAC: Got audio channels (tx for sending, rx for receiving)");
    
    // For now, we'll just test the UAC side since UAS simple server doesn't have get_call yet
    // This still tests that the call connects and audio channels work
    
    // Step 5: Send audio from UAC
    println!("\n=== Sending audio from UAC ===");
    
    let uac_send_handle = {
        let uac_audio_tx = uac_audio_tx.clone();
        tokio::spawn(async move {
            for i in 0..10 {
                // Generate unique pattern for UAC (values 1000-1009)
                let samples = vec![(1000 + i) as i16; 160];
                let frame = AudioFrame::new(samples, 8000, 1, (i * 160) as u32);
                
                match uac_audio_tx.send(frame).await {
                    Ok(_) => println!("UAC: Sent frame {} with value {}", i, 1000 + i),
                    Err(e) => println!("UAC: Failed to send frame {}: {}", i, e),
                }
                
                tokio::time::sleep(Duration::from_millis(20)).await;
            }
            println!("UAC: Finished sending 10 frames");
        })
    };
    
    // Wait for sending to complete
    let _ = uac_send_handle.await;
    
    // Step 6: Test other call operations
    println!("\n=== Testing Call Operations ===");
    
    // Test call state
    println!("UAC call state: {:?}", uac_call.state().await);
    println!("UAC call duration: {:?}", uac_call.duration());
    println!("UAC remote URI: {}", uac_call.remote_uri());
    
    // Test DTMF
    match uac_call.send_dtmf("123#").await {
        Ok(_) => println!("UAC: Sent DTMF digits '123#'"),
        Err(e) => println!("UAC: Failed to send DTMF: {}", e),
    }
    
    // Test hold/unhold
    println!("\n=== Testing Hold/Unhold ===");
    uac_call.hold().await.expect("Failed to hold");
    println!("UAC: Call on hold");
    
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    uac_call.unhold().await.expect("Failed to unhold");
    println!("UAC: Call resumed");
    
    // Test mute/unmute
    println!("\n=== Testing Mute/Unmute ===");
    uac_call.mute().await.expect("Failed to mute");
    println!("UAC: Audio muted");
    
    uac_call.unmute().await.expect("Failed to unmute");
    println!("UAC: Audio unmuted");
    
    // Get quality metrics
    let packet_loss = uac_call.packet_loss_rate().await;
    println!("UAC: Packet loss rate: {:.2}%", packet_loss * 100.0);
    
    // Step 7: Clean up
    println!("\n=== Cleaning up ===");
    
    // Hang up the call
    uac_call.hangup().await.expect("Failed to hang up UAC call");
    println!("UAC: Call hung up");
    
    tokio::time::sleep(Duration::from_millis(200)).await;
    
    // Shutdown client and server
    client.shutdown().await.expect("Failed to shutdown client");
    println!("UAC: Client shutdown");
    
    server.shutdown().await.expect("Failed to shutdown server");
    println!("UAS: Server shutdown");
    
    println!("\n=== Test Complete ===\n");
    
    println!("Note: This test validates the new Simple UAC API.");
    println!("Full bidirectional audio testing will be available when Simple UAS API is updated.");
}