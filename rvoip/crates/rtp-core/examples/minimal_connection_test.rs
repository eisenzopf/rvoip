/// Minimal connection test for diagnosing frame reception issues
///
/// This is a stripped-down version that focuses only on:
/// 1. Client connecting to server
/// 2. Sending raw UDP packets to the server
/// 3. Extensive logging of what happens at each stage

use std::net::{SocketAddr, IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::time::Duration;

use tokio::net::UdpSocket;
use tokio::sync::broadcast;
use tracing::Level;

use rvoip_rtp_core::api::common::frame::{MediaFrame, MediaFrameType};
use rvoip_rtp_core::api::server::transport::DefaultMediaTransportServer;
use rvoip_rtp_core::api::server::transport::MediaTransportServer;
use rvoip_rtp_core::api::server::config::{ServerConfig, ServerConfigBuilder};
use rvoip_rtp_core::api::common::config::SecurityMode;
use rvoip_rtp_core::api::server::security::ServerSecurityConfig;
use rvoip_rtp_core::traits::RtpEvent;
use rvoip_rtp_core::api::common::events::MediaEventCallback;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize detailed logging
    tracing_subscriber::fmt()
        .with_max_level(Level::TRACE)
        .init();
        
    println!("Minimal Connection Test - Diagnostics Mode");
    println!("=========================================\n");

    // Fixed ports for testing
    let server_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 20000);
    let client_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 10000);
    println!("Server address: {}", server_addr);
    println!("Client address: {}", client_addr);
    
    // Disable security for this test
    let mut server_security_config = ServerSecurityConfig::default();
    server_security_config.security_mode = SecurityMode::None;
    
    // Configure server with debugging options
    println!("\n[SETUP] Creating server configuration...");
    let server_config = ServerConfigBuilder::new()
        .local_address(server_addr)
        .default_payload_type(8) // G.711 µ-law
        .clock_rate(8000)
        .security_config(server_security_config)
        .jitter_buffer_size(50)
        .jitter_max_packet_age_ms(200)
        .enable_jitter_buffer(true)
        .build()?;
    
    // Create and start server
    println!("[SETUP] Creating server...");
    let server = DefaultMediaTransportServer::new(server_config).await?;
    
    // Set up event handlers before starting
    println!("[SETUP] Setting up event handlers...");
    server.on_event(Box::new(|event| {
        println!("[SERVER EVENT] {:?}", event);
    })).await?;
    
    server.on_client_connected(Box::new(|client_info| {
        println!("[SERVER] Client connected: {} from {}", client_info.id, client_info.address);
    })).await?;

    server.on_client_disconnected(Box::new(|client_info| {
        println!("[SERVER] Client disconnected: {} from {}", client_info.id, client_info.address);
    })).await?;
    
    // Start server
    println!("[SETUP] Starting server...");
    server.start().await?;
    println!("[SETUP] Server started successfully");
    
    // Get the actual server address after binding
    let actual_bind_address = match server.get_local_address().await {
        Ok(addr) => addr,
        Err(e) => {
            println!("[ERROR] Failed to get server address: {}", e);
            server_addr  // Fall back to the configured address
        }
    };
    println!("[SETUP] Server actually bound to: {}", actual_bind_address);
    
    // Create a task to monitor for received frames
    println!("[SETUP] Creating frame monitoring task...");
    let server_clone = server.clone();
    let monitor_task = tokio::spawn(async move {
        println!("[MONITOR] Frame monitoring started");
        
        // Get a persistent frame receiver instead of calling receive_frame() repeatedly
        let mut frame_receiver = server_clone.get_frame_receiver();
        
        for i in 0..20 {
            match tokio::time::timeout(Duration::from_millis(100), frame_receiver.recv()).await {
                Ok(Ok((client_id, frame))) => {
                    println!("[MONITOR] Frame received! Client ID: {}, Frame seq: {}, PT: {}, size: {} bytes", 
                             client_id, frame.sequence, frame.payload_type, frame.data.len());
                },
                Ok(Err(e)) => {
                    if i % 5 == 0 { // Only print every 5th error to reduce noise
                        println!("[MONITOR] Broadcast channel error: {}", e);
                    }
                },
                Err(_) => {
                    // Timeout - this is normal when no frames are available
                    if i % 5 == 0 { // Only print every 5th timeout to reduce noise
                        println!("[MONITOR] No frame available (timeout)");
                    }
                }
            }
            
            // Check client list to see if any clients are connected
            match server_clone.get_clients().await {
                Ok(clients) => {
                    if i % 5 == 0 || !clients.is_empty() { // Print every 5th update or whenever clients exist
                        println!("[MONITOR] Server has {} clients", clients.len());
                        for client in &clients {
                            println!("[MONITOR]   - Client: {}, Address: {}", client.id, client.address);
                        }
                    }
                },
                Err(e) => println!("[MONITOR] Error getting clients: {}", e),
            }
            
            // Small delay between iterations
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        
        println!("[MONITOR] Frame monitoring finished");
    });
    
    // Create a raw UDP socket client
    println!("\n[CLIENT] Creating raw UDP client socket on {}", client_addr);
    let client_socket = UdpSocket::bind(client_addr).await?;
    
    // Send packets directly to the server's actual bound address
    println!("[CLIENT] Sending test packets to server at {}", actual_bind_address);
    
    // Send several different packet types to test handling
    
    // 1. Proper RTP packet format
    println!("\n[CLIENT] Sending properly formatted RTP packet...");
    let rtp_packet = create_rtp_packet();
    client_socket.send_to(&rtp_packet, actual_bind_address).await?;
    println!("[CLIENT] Sent {} bytes of proper RTP data", rtp_packet.len());
    
    // 2. Almost RTP but wrong version
    tokio::time::sleep(Duration::from_millis(200)).await;
    println!("\n[CLIENT] Sending malformed RTP packet (wrong version)...");
    let mut malformed_rtp = vec![0x00, 0x08, 0x01, 0x23, 0x45, 0x67, 0x89, 0xAB, 0xCD, 0xEF];
    malformed_rtp.extend_from_slice(&[0u8; 160]); // Add G.711 sized payload
    client_socket.send_to(&malformed_rtp, actual_bind_address).await?;
    println!("[CLIENT] Sent {} bytes of malformed RTP data", malformed_rtp.len());
    
    // 3. Completely random data
    tokio::time::sleep(Duration::from_millis(200)).await;
    println!("\n[CLIENT] Sending completely random data...");
    let random_data = vec![0x12, 0x34, 0x56, 0x78, 0x9A, 0xBC, 0xDE, 0xF0, 0x12, 0x34, 0x56, 0x78];
    client_socket.send_to(&random_data, actual_bind_address).await?;
    println!("[CLIENT] Sent {} bytes of random data", random_data.len());
    
    // Wait for processing
    println!("\n[TEST] Waiting to see if packets are processed...");
    tokio::time::sleep(Duration::from_secs(2)).await;
    
    // Check server client list
    let clients = server.get_clients().await?;
    println!("\n[RESULTS] Server connected clients: {}", clients.len());
    for client in &clients {
        println!("[RESULTS]   - Client: {}, Address: {}", client.id, client.address);
    }
    
    // Clean up
    println!("\n[CLEANUP] Stopping server...");
    monitor_task.abort();
    
    // Sleep a bit to let logs catch up
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    if let Err(e) = server.stop().await {
        println!("[ERROR] Error stopping server: {}", e);
    } else {
        println!("[CLEANUP] Server stopped successfully");
    }
    
    // Add a wait to ensure clean shutdown
    tokio::time::sleep(Duration::from_millis(500)).await;
    println!("[CLEANUP] Test completed");
    
    Ok(())
}

// Helper function to create a minimally valid RTP packet
fn create_rtp_packet() -> Vec<u8> {
    let mut packet = Vec::new();
    
    // RTP Header
    packet.push(0x80);                  // Version=2, Padding=0, Extension=0, CSRC count=0
    packet.push(0x08);                  // Marker=0, Payload Type=8 (G.711 µ-law)
    packet.push(0x00);                  // Sequence number (high byte)
    packet.push(0x01);                  // Sequence number (low byte)
    packet.push(0x00);                  // Timestamp (byte 1)
    packet.push(0x00);                  // Timestamp (byte 2)
    packet.push(0x00);                  // Timestamp (byte 3)
    packet.push(0x01);                  // Timestamp (byte 4)
    packet.push(0x12);                  // SSRC (byte 1)
    packet.push(0x34);                  // SSRC (byte 2)
    packet.push(0x56);                  // SSRC (byte 3)
    packet.push(0x78);                  // SSRC (byte 4)
    
    // Add a sample payload (simulating G.711 audio)
    packet.extend_from_slice(&[0xFFu8; 160]); // 20ms of G.711 audio
    
    packet
} 