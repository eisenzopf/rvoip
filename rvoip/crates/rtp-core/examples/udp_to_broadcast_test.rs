/// This is a minimal test for UDP to broadcast channel bridging
/// 
/// It directly tests receiving UDP packets and forwarding them to a broadcast channel.

use std::net::{SocketAddr, IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::time::Duration;

use tokio::sync::{broadcast, Mutex};
use rvoip_rtp_core::transport::{UdpRtpTransport, RtpTransportConfig};
use rvoip_rtp_core::api::common::frame::{MediaFrame, MediaFrameType};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("UDP to Broadcast Channel Test");
    println!("=============================\n");

    // Create broadcast channel
    let (tx, _rx) = broadcast::channel::<(String, MediaFrame)>(100);
    println!("Created broadcast channel with capacity 100");
    
    // Create receiver
    let mut rx = tx.subscribe();
    println!("Created subscriber");
    
    // Create UDP transport
    let server_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 20000);
    let client_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 10000);
    
    let transport_config = RtpTransportConfig {
        local_rtp_addr: server_addr,
        local_rtcp_addr: None,
        symmetric_rtp: true,
        rtcp_mux: true,
        session_id: Some("test-transport".to_string()),
        use_port_allocator: false,
    };
    
    println!("Creating UDP transport on {}", server_addr);
    let transport = UdpRtpTransport::new(transport_config).await?;
    let transport = Arc::new(transport);
    
    // Subscribe to events
    let mut transport_events = transport.subscribe();
    println!("Subscribed to transport events");
    
    // Spawn event handler task
    let tx_clone = tx.clone();
    let event_task = tokio::spawn(async move {
        println!("Event handler task started");
        
        while let Ok(event) = transport_events.recv().await {
            match event {
                rvoip_rtp_core::traits::RtpEvent::MediaReceived { source, payload_type, timestamp, marker, payload, .. } => {
                    println!("Received RTP packet from {}, {} bytes, PT: {}", source, payload.len(), payload_type);
                    
                    // Create a simple frame from the data
                    let frame = MediaFrame {
                        frame_type: MediaFrameType::Audio,
                        data: payload.to_vec(),
                        timestamp,
                        sequence: 1, // We don't have sequence in the event
                        marker,
                        payload_type,
                        ssrc: 12345, // Default SSRC
                    };
                    
                    // Forward to broadcast channel
                    let client_id = format!("test-client-{}", source);
                    match tx_clone.send((client_id.clone(), frame)) {
                        Ok(receivers) => println!("Forwarded to {} receivers", receivers),
                        Err(e) => println!("No receivers for broadcast: {}", e),
                    }
                },
                rvoip_rtp_core::traits::RtpEvent::RtcpReceived { source, .. } => {
                    println!("Received RTCP packet from {}", source);
                },
                rvoip_rtp_core::traits::RtpEvent::Error(e) => {
                    println!("Transport error: {}", e);
                },
            }
        }
        
        println!("Event handler task ended");
    });
    
    // Create a simple UDP client to send test packets
    let client_socket = tokio::net::UdpSocket::bind(client_addr).await?;
    println!("Created client socket on {}", client_addr);
    
    // Send test packets
    println!("\nSending 3 test UDP packets...");
    for i in 1..=3 {
        let test_data = vec![i as u8; 20]; // Simple test data
        client_socket.send_to(&test_data, server_addr).await?;
        println!("Sent packet #{} ({} bytes) to {}", i, test_data.len(), server_addr);
        
        // Small delay
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    
    // Try to receive from the broadcast channel
    println!("\nTrying to receive from broadcast channel...");
    for _ in 0..3 {
        match tokio::time::timeout(Duration::from_millis(200), rx.recv()).await {
            Ok(Ok((client_id, frame))) => {
                println!("SUCCESS! Received frame from {}, {} bytes", client_id, frame.data.len());
            },
            Ok(Err(e)) => println!("Error receiving: {}", e),
            Err(_) => println!("Timeout waiting for message"),
        }
        
        // Small delay
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    
    // Clean up
    println!("\nTest completed");
    event_task.abort();
    
    Ok(())
} 