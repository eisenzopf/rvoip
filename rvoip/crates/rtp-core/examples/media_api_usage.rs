/// Example showing how media-core can integrate with rtp-core using the new API.
///
/// This example demonstrates:
/// 1. Creating client and server transport instances
/// 2. Sending and receiving media frames with broadcast channel
/// 3. Monitoring statistics

use std::net::{SocketAddr, IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::time::Duration;

use rvoip_rtp_core::api::common::frame::{MediaFrame, MediaFrameType};
use rvoip_rtp_core::api::common::stats::QualityLevel;
use rvoip_rtp_core::api::common::config::SecurityMode;

use rvoip_rtp_core::api::client::transport::{MediaTransportClient};
use rvoip_rtp_core::api::client::transport::DefaultMediaTransportClient;
use rvoip_rtp_core::api::client::config::{ClientConfig, ClientConfigBuilder};
use rvoip_rtp_core::api::client::security::ClientSecurityConfig;

use rvoip_rtp_core::api::server::transport::{MediaTransportServer};
use rvoip_rtp_core::api::server::transport::DefaultMediaTransportServer;
use rvoip_rtp_core::api::server::config::{ServerConfig, ServerConfigBuilder};
use rvoip_rtp_core::api::server::security::ServerSecurityConfig;

// Add direct access to these for debugging
use rvoip_rtp_core::traits::RtpEvent;
use rvoip_rtp_core::transport::RtpTransport;

async fn run_example() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging with more verbose output for debugging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::TRACE)
        .init();
    
    println!("RTP-Core Media API Example - DEBUG MODE");
    println!("========================================\n");
    println!("This example will automatically terminate after 15 seconds\n");

    // Fixed ports for testing
    let client_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 10000);
    let server_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 20000);
    println!("Using fixed addresses - Server: {}, Client: {}", server_addr, client_addr);
    
    // Set up auto-termination after 15 seconds
    let timeout_handle = tokio::spawn(async {
        tokio::time::sleep(Duration::from_secs(15)).await;
        println!("\n[TIMEOUT] 15-second timeout reached. Terminating example...");
        std::process::exit(0); // Force exit the process
    });
    
    // Configure security - disable for this test
    let mut client_security_config = ClientSecurityConfig::default();
    client_security_config.security_mode = SecurityMode::None; // Disable security
    
    let mut server_security_config = ServerSecurityConfig::default();
    server_security_config.security_mode = SecurityMode::None; // Disable security
    
    // Configure server
    println!("[DEBUG] Configuring server...");
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
    println!("[DEBUG] Creating and starting server...");
    let server = DefaultMediaTransportServer::new(server_config).await?;
    
    // Set up event handlers before starting
    server.on_event(Box::new(|event| {
        println!("[SERVER EVENT] {:?}", event);
    })).await?;
    
    server.on_client_connected(Box::new(|client_info| {
        println!("[SERVER] Client connected: {} from {}", client_info.id, client_info.address);
    })).await?;
    
    server.start().await?;
    println!("[DEBUG] Server started successfully");
    
    // Get actual server address
    let actual_server_addr = match server.get_local_address().await {
        Ok(addr) => addr,
        Err(e) => {
            println!("[WARNING] Could not get actual server address: {}", e);
            server_addr // Fall back to configured address
        }
    };
    println!("[DEBUG] Server actually bound to: {}", actual_server_addr);
    
    // Give server a moment to fully initialize
    tokio::time::sleep(Duration::from_millis(200)).await;
    
    // Configure client
    println!("[DEBUG] Configuring client...");
    let client_config = ClientConfigBuilder::new()
        .remote_address(actual_server_addr) // Use the actual server address
        .default_payload_type(8) // G.711 µ-law
        .clock_rate(8000)
        .security_config(client_security_config)
        .jitter_buffer_size(50)
        .jitter_max_packet_age_ms(200)
        .enable_jitter_buffer(true)
        .build();
    
    // Create client and connect to server
    println!("[DEBUG] Creating client...");
    let client = DefaultMediaTransportClient::new(client_config).await?;
    
    client.on_event(Box::new(|event| {
        println!("[CLIENT EVENT] {:?}", event);
    })).await?;
    
    client.on_connect(Box::new(|| {
        println!("[CLIENT] Connected to server");
    })).await?;
    
    println!("[DEBUG] Connecting client to server at {}...", actual_server_addr);
    client.connect().await?;
    
    // Wait for connection
    println!("[DEBUG] Waiting for connection...");
    tokio::time::sleep(Duration::from_millis(300)).await;
    
    // Verify client is connected
    if client.is_connected().await? {
        println!("[DEBUG] Client successfully connected to server");
    } else {
        println!("[ERROR] Client failed to connect to server");
        return Err("Connection failed".into());
    }
    
    // Display current clients on server
    let clients = server.get_clients().await?;
    println!("[DEBUG] Server has {} connected clients:", clients.len());
    for client in &clients {
        println!("  - Client ID: {}, Address: {}", client.id, client.address);
    }
    
    if clients.is_empty() {
        println!("[WARNING] Server shows no connected clients! This is likely the root issue.");
        println!("The server's event handler for RtpEvent::MediaReceived may not be firing.");
    }
    
    // Create test audio frame (very small data for testing)
    let audio_frame = MediaFrame {
        frame_type: MediaFrameType::Audio,
        data: vec![1, 2, 3, 4, 5, 6, 7, 8],
        timestamp: 1000,
        sequence: 1,
        marker: false,
        payload_type: 8, // PCMA
        ssrc: 12345,
        csrcs: Vec::new(),
    };
    
    // Send frames
    println!("\n[DEBUG] ======= STARTING FRAME TRANSMISSION TEST =======");
    println!("[DEBUG] Sending 3 test frames from client to server...");
    for i in 0..3 {
        let mut frame = audio_frame.clone();
        frame.sequence = i + 1;
        frame.timestamp = 1000 + (i as u32 * 160);
        
        println!("[DEBUG] Sending frame #{} with timestamp {}", i+1, frame.timestamp);
        match client.send_frame(frame).await {
            Ok(_) => println!("[DEBUG] Frame #{} sent successfully", i+1),
            Err(e) => println!("[ERROR] Failed to send frame #{}: {}", i+1, e),
        }
        
        // Give time for network and processing
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    
    // Display updated clients after sending frames
    let clients_after = server.get_clients().await?;
    println!("\n[DEBUG] Server has {} clients after sending frames:", clients_after.len());
    for client in &clients_after {
        println!("  - Client ID: {}, Address: {}", client.id, client.address);
    }
    
    // Try to receive frames on server
    println!("\n[DEBUG] Testing server.receive_frame()...");
    println!("[DEBUG] Attempting to receive frames on server (this should get frames from the broadcast channel)...");
    
    let mut frames_received = 0;
    let start_time = std::time::Instant::now();
    
    while frames_received < 3 && start_time.elapsed() < Duration::from_secs(1) {
        match server.receive_frame().await {
            Ok((client_id, frame)) => {
                frames_received += 1;
                println!("[SUCCESS] Server received frame #{} from client {}: seq={}, ts={}",
                         frames_received, client_id, frame.sequence, frame.timestamp);
            },
            Err(e) => {
                println!("[DEBUG] receive_frame() returned error: {}", e);
                tokio::time::sleep(Duration::from_millis(100)).await;
            }
        }
    }
    
    if frames_received > 0 {
        println!("\n[SUCCESS] Test PASSED! Received {} frames", frames_received);
    } else {
        println!("\n[FAILURE] Test FAILED! No frames received on server");
        println!("This indicates the broadcast channel isn't receiving frames from the transport layer.");
    }
    
    // Clean up
    println!("\n[DEBUG] Cleaning up...");
    
    println!("[DEBUG] Disconnecting client...");
    if let Err(e) = client.disconnect().await {
        println!("[ERROR] Error disconnecting client: {}", e);
    }
    
    // Allow disconnection to complete
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    println!("[DEBUG] Stopping server...");
    if let Err(e) = server.stop().await {
        println!("[ERROR] Error stopping server: {}", e);
    }
    
    // Explicitly abort the main task handle from the server
    if let Ok(clients) = server.get_clients().await {
        println!("[DEBUG] Server still has {} clients after stop call", clients.len());
    }
    
    // Force additional sleep to ensure resources are cleaned up
    println!("[DEBUG] Waiting for resources to clean up...");
    tokio::time::sleep(Duration::from_millis(500)).await;
    
    // Cancel the timeout since we're finishing normally
    timeout_handle.abort();
    
    println!("[DEBUG] Example completed");
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    match run_example().await {
        Ok(_) => {
            println!("Example completed successfully");
            Ok(())
        },
        Err(e) => {
            eprintln!("Error running example: {}", e);
            Err(e)
        }
    }
} 