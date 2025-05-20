/// RTCP Reports API Example
///
/// This example demonstrates how to use the RTCP Sender/Receiver Reports API
/// in the client and server transport interfaces.

use std::net::{SocketAddr, IpAddr, Ipv4Addr};
use std::time::Duration;

use rvoip_rtp_core::api::common::frame::{MediaFrame, MediaFrameType};
use rvoip_rtp_core::api::common::config::SecurityMode;
use rvoip_rtp_core::api::common::events::MediaTransportEvent;
use rvoip_rtp_core::api::client::transport::MediaTransportClient;
use rvoip_rtp_core::api::client::config::ClientConfigBuilder;
use rvoip_rtp_core::api::client::security::ClientSecurityConfig;
use rvoip_rtp_core::api::server::transport::MediaTransportServer;
use rvoip_rtp_core::api::server::config::ServerConfigBuilder;
use rvoip_rtp_core::api::server::security::ServerSecurityConfig;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Set a global timeout for the entire example (15 seconds max)
    match tokio::time::timeout(Duration::from_secs(15), run_example()).await {
        Ok(result) => result,
        Err(_) => {
            eprintln!("\n\nTest timed out after 15 seconds");
            Ok(())
        }
    }
}

async fn run_example() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();
    
    println!("RTCP Reports API Example");
    println!("=======================\n");
    
    // Create a server
    println!("1. Creating server");
    let server = create_server().await?;
    
    // Start the server
    println!("2. Starting server");
    server.start().await?;
    
    // Get the server's local address
    let server_addr = server.get_local_address().await?;
    println!("   Server bound to: {}", server_addr);
    
    // Create a client
    println!("3. Creating client");
    let client = create_client(server_addr).await?;
    
    // Connect client to server
    println!("4. Connecting client to server");
    client.connect().await?;
    println!("   Client connected successfully");
    
    // Get client's local address
    let client_addr = client.get_local_address().await?;
    println!("   Client bound to: {}", client_addr);
    
    // Register event handlers
    println!("5. Setting up event handlers");
    setup_server_events(&server).await?;
    setup_client_events(&client).await?;
    
    // Set RTCP interval (faster than default for demonstration)
    println!("6. Setting RTCP reporting interval to 1 second");
    client.set_rtcp_interval(Duration::from_secs(1)).await?;
    server.set_rtcp_interval(Duration::from_secs(1)).await?;
    
    // Send some media frames in both directions
    println!("7. Sending media frames");
    
    // Send 10 frames from client to server
    for i in 0..10 {
        let frame = create_test_frame(i);
        client.send_frame(frame).await?;
        
        // Brief pause between frames
        tokio::time::sleep(Duration::from_millis(50)).await;
    }
    
    // Wait a bit for frames to be processed
    println!("8. Waiting for frames to be processed...");
    tokio::time::sleep(Duration::from_millis(500)).await;
    
    // Manually trigger RTCP reports
    println!("9. Manually triggering RTCP reports");
    println!("   Client sending RTCP Receiver Report");
    client.send_rtcp_receiver_report().await?;
    
    println!("   Server sending RTCP Sender Report");
    server.send_rtcp_sender_report().await?;
    
    // Wait a bit for reports to be exchanged
    tokio::time::sleep(Duration::from_millis(500)).await;
    
    // Get RTCP stats
    println!("10. Getting RTCP statistics");
    let client_stats = client.get_rtcp_stats().await?;
    println!("    Client RTCP stats:");
    println!("    - Jitter: {:.2} ms", client_stats.jitter_ms);
    println!("    - Packet loss: {:.2}%", client_stats.packet_loss_percent);
    if let Some(rtt) = client_stats.round_trip_time_ms {
        println!("    - Round trip time: {:.2} ms", rtt);
    } else {
        println!("    - Round trip time: not available yet");
    }
    
    // Get server stats for all clients
    let server_stats = server.get_rtcp_stats().await?;
    println!("    Server RTCP stats (all clients):");
    println!("    - Jitter: {:.2} ms", server_stats.jitter_ms);
    println!("    - Packet loss: {:.2}%", server_stats.packet_loss_percent);
    if let Some(rtt) = server_stats.round_trip_time_ms {
        println!("    - Round trip time: {:.2} ms", rtt);
    } else {
        println!("    - Round trip time: not available yet");
    }
    
    // Get list of clients
    println!("11. Getting client list from server");
    let clients = server.get_clients().await?;
    println!("    Server has {} connected clients:", clients.len());
    
    // For each client, get individual RTCP stats
    for client_info in &clients {
        println!("    Client {} stats:", client_info.id);
        match server.get_client_rtcp_stats(&client_info.id).await {
            Ok(stats) => {
                println!("    - Jitter: {:.2} ms", stats.jitter_ms);
                println!("    - Packet loss: {:.2}%", stats.packet_loss_percent);
                if let Some(rtt) = stats.round_trip_time_ms {
                    println!("    - Round trip time: {:.2} ms", rtt);
                } else {
                    println!("    - Round trip time: not available yet");
                }
            },
            Err(e) => {
                println!("    Error getting RTCP stats: {}", e);
            }
        }
    }
    
    // Demonstrate RTCP report to specific client
    if !clients.is_empty() {
        let first_client_id = &clients[0].id;
        println!("12. Sending RTCP Receiver Report to specific client ({})", first_client_id);
        server.send_rtcp_receiver_report_to_client(first_client_id).await?;
    }
    
    // Cleanup
    println!("13. Cleaning up");
    client.disconnect().await?;
    server.stop().await?;
    
    println!("\nExample completed successfully");
    Ok(())
}

/// Create a server with default configuration and no security
async fn create_server() -> Result<impl MediaTransportServer, Box<dyn std::error::Error>> {
    // Create server config
    let mut server_config = ServerConfigBuilder::new()
        .local_address(SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0))
        .build()?;
    
    // Disable security for this example
    let mut security_config = ServerSecurityConfig::default();
    security_config.security_mode = SecurityMode::None;
    server_config.security_config = security_config;
    
    // Create server
    let server = rvoip_rtp_core::api::create_server(server_config).await?;
    
    Ok(server)
}

/// Create a client connected to the specified server address
async fn create_client(server_addr: SocketAddr) -> Result<impl MediaTransportClient, Box<dyn std::error::Error>> {
    // Create client config
    let mut client_config = ClientConfigBuilder::new()
        .remote_address(server_addr)
        .build();
    
    // Disable security for this example
    let mut security_config = ClientSecurityConfig::default();
    security_config.security_mode = SecurityMode::None;
    client_config.security_config = security_config;
    
    // Create client
    let client = rvoip_rtp_core::api::create_client(client_config).await?;
    
    Ok(client)
}

/// Set up event handlers for the server
async fn setup_server_events(server: &impl MediaTransportServer) -> Result<(), Box<dyn std::error::Error>> {
    server.on_event(Box::new(|event| {
        match event {
            MediaTransportEvent::Connected => {
                println!("[SERVER EVENT] Client connected");
            },
            MediaTransportEvent::Disconnected => {
                println!("[SERVER EVENT] Client disconnected");
            },
            MediaTransportEvent::FrameReceived(frame) => {
                println!("[SERVER EVENT] Received frame: seq={}, size={} bytes", 
                         frame.sequence, frame.data.len());
            },
            MediaTransportEvent::RtcpReport { jitter, packet_loss, round_trip_time } => {
                println!("[SERVER EVENT] RTCP Report - Jitter: {:.2}ms, Loss: {:.2}%, RTT: {:?}ms", 
                         jitter, packet_loss * 100.0, round_trip_time.map(|rtt| rtt.as_millis()));
            },
            MediaTransportEvent::Error(err) => {
                println!("[SERVER EVENT] Error: {}", err);
            },
            _ => {}
        }
    })).await?;
    
    Ok(())
}

/// Set up event handlers for the client
async fn setup_client_events(client: &impl MediaTransportClient) -> Result<(), Box<dyn std::error::Error>> {
    client.on_event(Box::new(|event| {
        match event {
            MediaTransportEvent::Connected => {
                println!("[CLIENT EVENT] Connected to server");
            },
            MediaTransportEvent::Disconnected => {
                println!("[CLIENT EVENT] Disconnected from server");
            },
            MediaTransportEvent::FrameReceived(frame) => {
                println!("[CLIENT EVENT] Received frame: seq={}, size={} bytes", 
                         frame.sequence, frame.data.len());
            },
            MediaTransportEvent::RtcpReport { jitter, packet_loss, round_trip_time } => {
                println!("[CLIENT EVENT] RTCP Report - Jitter: {:.2}ms, Loss: {:.2}%, RTT: {:?}ms", 
                         jitter, packet_loss * 100.0, round_trip_time.map(|rtt| rtt.as_millis()));
            },
            MediaTransportEvent::Error(err) => {
                println!("[CLIENT EVENT] Error: {}", err);
            },
            _ => {}
        }
    })).await?;
    
    Ok(())
}

/// Create a test media frame
fn create_test_frame(sequence: u16) -> MediaFrame {
    MediaFrame {
        frame_type: MediaFrameType::Audio,
        data: vec![1, 2, 3, 4, 5, 6, 7, 8],
        timestamp: 1000 + (sequence as u32 * 160), // 20ms samples at 8kHz
        sequence,
        marker: sequence == 0, // Mark first packet
        payload_type: 8, // G.711 μ-law
        ssrc: 12345,
    }
} 