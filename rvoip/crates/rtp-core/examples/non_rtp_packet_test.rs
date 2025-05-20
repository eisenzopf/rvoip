/// This example tests the handling of non-RTP packets in the UdpRtpTransport
/// 
/// It demonstrates that even malformed or non-RTP formatted packets are properly 
/// processed and generate MediaReceived events.

use std::net::{SocketAddr, IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::time::Duration;

use tokio::net::UdpSocket;
use tokio::sync::broadcast;

use rvoip_rtp_core::transport::{UdpRtpTransport, RtpTransportConfig};
use rvoip_rtp_core::traits::RtpEvent;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Non-RTP Packet Test");
    println!("==================\n");
    
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
    
    // Subscribe to events
    let mut transport_events = transport.subscribe();
    println!("Subscribed to transport events");
    
    // Spawn event handler task
    let event_handler = tokio::spawn(async move {
        println!("Event handler task started");
        
        // Process events with a timeout
        let mut event_count = 0;
        
        while event_count < 3 {
            match tokio::time::timeout(Duration::from_millis(500), transport_events.recv()).await {
                Ok(Ok(event)) => {
                    match event {
                        RtpEvent::MediaReceived { source, payload_type, timestamp, marker, payload, .. } => {
                            event_count += 1;
                            println!("MediaReceived #{} - {} bytes from {}", 
                                     event_count, payload.len(), source);
                            println!("  Payload Type: {}", payload_type);
                            println!("  Timestamp: {}", timestamp);
                            println!("  Marker: {}", marker);
                            println!("  First few bytes: {:?}", &payload[..std::cmp::min(payload.len(), 8)]);
                        },
                        RtpEvent::RtcpReceived { source, .. } => {
                            println!("RTCP received from {}", source);
                        },
                        RtpEvent::Error(e) => {
                            println!("Transport error: {}", e);
                        },
                    }
                },
                Ok(Err(e)) => {
                    println!("Error receiving event: {}", e);
                    break;
                },
                Err(_) => {
                    println!("Timeout waiting for events");
                    break;
                }
            }
        }
        
        println!("Event handler task ended");
    });
    
    // Create a simple UDP client to send test packets
    let client_socket = UdpSocket::bind(client_addr).await?;
    println!("Created client socket on {}", client_addr);
    
    // Send test packets - a mix of valid and invalid RTP packets
    println!("\nSending 3 test UDP packets (non-RTP format)...");

    // Packet 1: Completely random data
    let random_data = vec![0xAA, 0xBB, 0xCC, 0xDD, 0xEE, 0xFF, 0x11, 0x22, 0x33, 0x44];
    client_socket.send_to(&random_data, server_addr).await?;
    println!("Sent random data packet ({} bytes)", random_data.len());
    tokio::time::sleep(Duration::from_millis(50)).await;
    
    // Packet 2: Almost RTP but wrong version (should fail RTP parsing)
    // First byte: 0x00 (version 0, no padding, no extension, no CSRC)
    let mut almost_rtp = vec![0x00, 0x08, 0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF, 0x00, 0x01];
    client_socket.send_to(&almost_rtp, server_addr).await?;
    println!("Sent almost-RTP packet with wrong version ({} bytes)", almost_rtp.len());
    tokio::time::sleep(Duration::from_millis(50)).await;
    
    // Packet 3: Truncated RTP header (should fail RTP parsing)
    let truncated_rtp = vec![0x80, 0x08, 0x01, 0x23, 0x45, 0x67];
    client_socket.send_to(&truncated_rtp, server_addr).await?;
    println!("Sent truncated RTP header ({} bytes)", truncated_rtp.len());
    tokio::time::sleep(Duration::from_millis(50)).await;
    
    // Wait for the event handler to process events
    println!("\nWaiting for events to be processed...");
    tokio::time::sleep(Duration::from_millis(1000)).await;
    
    // Clean up
    println!("\nTest completed");
    event_handler.abort();
    
    Ok(())
} 