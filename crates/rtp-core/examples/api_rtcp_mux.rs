/// API-based RTCP-MUX Configuration Example
///
/// This example demonstrates how to use the RTCP multiplexing (RFC 5761) option
/// in the client and server API. It creates a server and client with different
/// RTCP-MUX configurations and shows how they interact.
///
/// Expected runtime: ~5-10 seconds

use std::net::{SocketAddr, IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::time::Duration;

use rvoip_rtp_core::api::common::frame::{MediaFrame, MediaFrameType};
use rvoip_rtp_core::api::common::config::SecurityMode;
use rvoip_rtp_core::api::client::transport::MediaTransportClient;
use rvoip_rtp_core::api::client::config::ClientConfigBuilder;
use rvoip_rtp_core::api::client::security::ClientSecurityConfig;
use rvoip_rtp_core::api::server::transport::MediaTransportServer;
use rvoip_rtp_core::api::server::config::ServerConfigBuilder;
use rvoip_rtp_core::api::server::security::ServerSecurityConfig;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Set a global timeout for the entire example (15 seconds max)
    match tokio::time::timeout(Duration::from_secs(15), async {
        // Set up logging
        tracing_subscriber::fmt()
            .with_max_level(tracing::Level::INFO) // Changed from DEBUG to INFO to reduce log noise
            .init();
        
        println!("API-based RTCP-MUX Configuration Example");
        println!("=======================================\n");
        
        // Demonstrate different server configurations
        println!("1. Creating server WITH RTCP-MUX enabled");
        let server_with_mux = create_server(true).await?;
        
        println!("2. Creating server WITH RTCP-MUX disabled");
        let server_without_mux = create_server(false).await?;
        
        // Get the actual addresses the servers are bound to
        let server_mux_addr = server_with_mux.get_local_address().await?;
        let server_no_mux_addr = server_without_mux.get_local_address().await?;
        
        println!("\nServer with RTCP-MUX is bound to: {}", server_mux_addr);
        println!("Server without RTCP-MUX is bound to: {}", server_no_mux_addr);
        
        // Demonstrate client connection to server with matching RTCP-MUX setting
        println!("\n3. Creating client WITH RTCP-MUX enabled and connecting to matching server");
        let client_with_mux = create_client(true, server_mux_addr).await?;
        
        // Connect the client to the server first
        println!("Connecting client to server...");
        client_with_mux.connect().await?;
        
        // Print the local address of the client after connecting
        let client_mux_addr = client_with_mux.get_local_address().await?;
        println!("Client with RTCP-MUX is bound to: {}", client_mux_addr);
        
        // Send a test frame
        let frame = create_test_frame();
        println!("Sending test frame from client to server...");
        client_with_mux.send_frame(frame).await?;
        
        // Try to receive the frame on the server
        println!("Waiting for frame on server...");
        match tokio::time::timeout(Duration::from_millis(250), server_with_mux.receive_frame()).await {
            Ok(Ok((client_id, received_frame))) => {
                println!("Server received frame from client {}", client_id);
                println!("  Frame info: type={:?}, seq={}, size={} bytes",
                        received_frame.frame_type,
                        received_frame.sequence,
                        received_frame.data.len());
            },
            Ok(Err(e)) => {
                println!("Error receiving frame: {}", e);
            },
            Err(_) => {
                println!("Timeout waiting for frame");
            }
        }
        
        // Disconnect client
        println!("\nDisconnecting client...");
        client_with_mux.disconnect().await?;
        
        // Now demonstrate client/server with mismatched RTCP-MUX settings
        println!("\n4. Creating client WITHOUT RTCP-MUX enabled and connecting to server WITH RTCP-MUX");
        let client_without_mux = create_client(false, server_mux_addr).await?;
        
        // Try to connect the client to the server
        println!("Connecting client to server...");
        match client_without_mux.connect().await {
            Ok(_) => {
                println!("Client successfully connected to server (despite RTCP-MUX mismatch)");
                
                // Print the local address of the client after connecting
                let client_no_mux_addr = client_without_mux.get_local_address().await?;
                println!("Client without RTCP-MUX is bound to: {}", client_no_mux_addr);
                
                // Disconnect client
                println!("Disconnecting client...");
                client_without_mux.disconnect().await?;
            },
            Err(e) => {
                println!("Client failed to connect to server: {}", e);
            }
        }
        
        // Simplified WebRTC test (just one example instead of full testing)
        println!("\n5. Testing WebRTC config (with RTCP-MUX enabled by default)");
        
        // Create WebRTC server
        let webrtc_server_config = ServerConfigBuilder::webrtc()
            .local_address(SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0))
            .build()?;
        
        // Disable security for the server
        let mut server_config = webrtc_server_config;
        let mut server_security = ServerSecurityConfig::default();
        server_security.security_mode = SecurityMode::None;
        server_config.security_config = server_security;
        
        let webrtc_server = rvoip_rtp_core::api::create_server(server_config).await?;
        webrtc_server.start().await?;
        
        let webrtc_server_addr = webrtc_server.get_local_address().await?;
        println!("WebRTC server bound to: {}", webrtc_server_addr);
        
        // Create WebRTC client with disabled security
        let webrtc_client_config = ClientConfigBuilder::webrtc()
            .remote_address(webrtc_server_addr)
            .build();
            
        // Disable security for the client
        let mut client_config = webrtc_client_config;
        let mut client_security = ClientSecurityConfig::default();
        client_security.security_mode = SecurityMode::None;
        client_config.security_config = client_security;
        
        let webrtc_client = rvoip_rtp_core::api::create_client(client_config).await?;
        
        // Connect WebRTC client to server
        println!("Connecting WebRTC client to server...");
        webrtc_client.connect().await?;
        
        let webrtc_client_addr = webrtc_client.get_local_address().await?;
        println!("WebRTC client bound to: {}", webrtc_client_addr);
        
        // Send a test frame
        let frame = create_test_frame();
        println!("Sending test frame from WebRTC client to server...");
        webrtc_client.send_frame(frame).await?;
        
        // Disconnect and clean up
        println!("\nDisconnecting WebRTC client...");
        webrtc_client.disconnect().await?;
        
        println!("\nStopping all servers...");
        server_with_mux.stop().await?;
        server_without_mux.stop().await?;
        webrtc_server.stop().await?;
        
        println!("\nExample completed successfully");
        Ok(())
    }).await {
        Ok(result) => result,
        Err(_) => {
            println!("\n\nTest timed out after 15 seconds");
            println!("Terminating example");
            Ok(())
        }
    }
}

/// Helper function to create a server with specified RTCP-MUX setting
async fn create_server(rtcp_mux: bool) -> Result<impl MediaTransportServer, Box<dyn std::error::Error>> {
    // Create server config with specified RTCP-MUX setting
    let mut server_config = ServerConfigBuilder::new()
        .local_address(SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0))
        .rtcp_mux(rtcp_mux)
        .build()?;
    
    // Disable security
    let mut security_config = ServerSecurityConfig::default();
    security_config.security_mode = SecurityMode::None;
    server_config.security_config = security_config;
    
    // Create server
    let server = rvoip_rtp_core::api::create_server(server_config).await?;
    
    // Start server
    server.start().await?;
    
    Ok(server)
}

/// Helper function to create a client with specified RTCP-MUX setting
async fn create_client(rtcp_mux: bool, server_addr: SocketAddr) -> Result<impl MediaTransportClient, Box<dyn std::error::Error>> {
    // Create client config with specified RTCP-MUX setting
    let mut client_config = ClientConfigBuilder::new()
        .remote_address(server_addr)
        .rtcp_mux(rtcp_mux)
        .build();
    
    // Disable security
    let mut security_config = ClientSecurityConfig::default();
    security_config.security_mode = SecurityMode::None;
    client_config.security_config = security_config;
    
    // Create client
    let client = rvoip_rtp_core::api::create_client(client_config).await?;
    
    Ok(client)
}

/// Helper function to create a test media frame
fn create_test_frame() -> MediaFrame {
    MediaFrame {
        frame_type: MediaFrameType::Audio,
        data: vec![1, 2, 3, 4, 5],
        timestamp: 1000,
        sequence: 1,
        marker: false,
        payload_type: 8, // G.711 μ-law
        ssrc: 12345,
        csrcs: Vec::new(),
    }
} 