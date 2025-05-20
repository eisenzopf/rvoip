/// Testing the fixed implementation of `receive_frame` in the server transport
///
/// This minimal example shows how to fix the receive_frame implementation to use
/// broadcast channel correctly.

use std::net::{SocketAddr, IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::broadcast;
use rvoip_rtp_core::api::common::frame::{MediaFrame, MediaFrameType};
use rvoip_rtp_core::api::common::error::MediaTransportError;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("RTP Broadcast Channel Fix");
    println!("========================\n");

    // Create a broadcast channel for passing frames 
    let (tx, _) = broadcast::channel::<(String, MediaFrame)>(100);
    println!("Created broadcast channel with capacity 100");
    
    // Create two receivers
    let mut rx1 = tx.subscribe();
    let mut rx2 = tx.subscribe();
    println!("Created 2 subscribers to test broadcast");
    
    // Create a task to send frames to the broadcast channel
    let tx_clone = tx.clone();
    let sender_task = tokio::spawn(async move {
        for i in 1..=5 {
            let frame = MediaFrame {
                frame_type: MediaFrameType::Audio,
                data: vec![i as u8; 20],
                timestamp: i * 1000,
                sequence: i as u16,
                marker: false,
                payload_type: 8,
                ssrc: 12345,
            };
            
            let client_id = format!("test-client-{}", i);
            
            println!("Sending frame {} to broadcast channel", i);
            match tx_clone.send((client_id, frame)) {
                Ok(receivers) => println!("Frame {} sent to {} receivers", i, receivers),
                Err(e) => println!("Error sending frame {}: {}", i, e),
            }
            
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
    });
    
    // Create a function that models the fixed receive_frame behavior
    async fn receive_frame(rx: &mut broadcast::Receiver<(String, MediaFrame)>) 
        -> Result<(String, MediaFrame), MediaTransportError> 
    {
        // Wait for a frame with a timeout
        match tokio::time::timeout(Duration::from_millis(500), rx.recv()).await {
            Ok(Ok(frame)) => {
                // Success - we got a frame
                Ok(frame)
            },
            Ok(Err(e)) => {
                // Broadcast channel error
                Err(MediaTransportError::Transport(format!("Broadcast error: {}", e)))
            },
            Err(_) => {
                // Timeout
                Err(MediaTransportError::Timeout("No frame received within timeout".to_string()))
            }
        }
    }
    
    // Wait a bit for the sender to start
    tokio::time::sleep(Duration::from_millis(50)).await;
    
    // Now try to receive on both receivers - they should get the same frames
    println!("\nReceiver 1 attempting to receive:");
    for _ in 0..3 {
        match receive_frame(&mut rx1).await {
            Ok((client_id, frame)) => {
                println!("Receiver 1 got frame from {}: seq={}, ts={}", 
                    client_id, frame.sequence, frame.timestamp);
            },
            Err(e) => println!("Receiver 1 error: {}", e),
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    
    println!("\nReceiver 2 attempting to receive:");
    for _ in 0..3 {
        match receive_frame(&mut rx2).await {
            Ok((client_id, frame)) => {
                println!("Receiver 2 got frame from {}: seq={}, ts={}", 
                    client_id, frame.sequence, frame.timestamp);
            },
            Err(e) => println!("Receiver 2 error: {}", e),
        }
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    
    // Clean up
    println!("\nTest completed");
    sender_task.abort();
    
    Ok(())
} 