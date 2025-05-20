/// This is a minimal test for the broadcast channel functionality
/// 
/// It directly tests the broadcast channel without the complexity
/// of client/server communications to isolate the issue.

use std::time::Duration;
use tokio::sync::broadcast;
use rvoip_rtp_core::api::common::frame::{MediaFrame, MediaFrameType};

#[tokio::main]
async fn main() {
    println!("Broadcast Channel Direct Test");
    println!("============================\n");

    // Create a broadcast channel
    let (tx, _rx) = broadcast::channel::<(String, MediaFrame)>(100);
    println!("Created broadcast channel with capacity 100");
    
    // Create a few receivers
    let mut rx1 = tx.subscribe();
    let mut rx2 = tx.subscribe();
    println!("Created 2 subscribers (rx1, rx2)");
    
    // Create a test frame
    let frame = MediaFrame {
        frame_type: MediaFrameType::Audio,
        data: vec![1, 2, 3, 4, 5],
        timestamp: 1000,
        sequence: 1,
        marker: false,
        payload_type: 8,
        ssrc: 12345,
    };
    
    // Spawn task to send messages to the channel
    println!("Sending 3 test frames to the channel...");
    for i in 1..=3 {
        let mut frame_copy = frame.clone();
        frame_copy.sequence = i;
        let client_id = format!("test-client-{}", i);
        
        match tx.send((client_id.clone(), frame_copy)) {
            Ok(receivers) => println!("Frame {} sent successfully to {} receivers", i, receivers),
            Err(e) => println!("Error sending frame {}: {}", i, e),
        }
        
        // Small delay between sends
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    
    // Print sender metrics
    println!("\nSender metrics:");
    println!("Receiver count: {}", tx.receiver_count());
    
    // Try to receive on rx1
    println!("\nTrying to receive on rx1...");
    match tokio::time::timeout(Duration::from_millis(100), rx1.recv()).await {
        Ok(result) => match result {
            Ok((client_id, frame)) => {
                println!("SUCCESS! rx1 received frame from {}: seq={}", client_id, frame.sequence);
            },
            Err(e) => println!("Error receiving on rx1: {}", e),
        },
        Err(_) => println!("Timeout waiting for message on rx1"),
    }
    
    // Try to receive on rx2
    println!("\nTrying to receive on rx2...");
    match tokio::time::timeout(Duration::from_millis(100), rx2.recv()).await {
        Ok(result) => match result {
            Ok((client_id, frame)) => {
                println!("SUCCESS! rx2 received frame from {}: seq={}", client_id, frame.sequence);
            },
            Err(e) => println!("Error receiving on rx2: {}", e),
        },
        Err(_) => println!("Timeout waiting for message on rx2"),
    }
    
    // Check if receivers can receive more messages
    println!("\nTrying to receive more messages on rx1...");
    match tokio::time::timeout(Duration::from_millis(100), rx1.recv()).await {
        Ok(result) => match result {
            Ok((client_id, frame)) => {
                println!("SUCCESS! rx1 received another frame from {}: seq={}", client_id, frame.sequence);
            },
            Err(e) => println!("Error receiving more on rx1: {}", e),
        },
        Err(_) => println!("Timeout waiting for more messages on rx1"),
    }
    
    println!("\nBroadcast channel test completed");
} 