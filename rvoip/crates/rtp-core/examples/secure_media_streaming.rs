/// Example showing secure media streaming with DTLS-SRTP
///
/// This example demonstrates setting up a client and server with 
/// DTLS-SRTP security for secure media transmission.

use std::net::{SocketAddr, IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::time::Duration;

use rvoip_rtp_core::api::client::transport::{MediaTransportClient, DefaultMediaTransportClient};
use rvoip_rtp_core::api::server::transport::{MediaTransportServer, DefaultMediaTransportServer};
use rvoip_rtp_core::api::client::config::ClientConfigBuilder;
use rvoip_rtp_core::api::server::config::ServerConfigBuilder;
use rvoip_rtp_core::api::common::frame::{MediaFrame, MediaFrameType};
use rvoip_rtp_core::api::common::events::MediaTransportEvent;
use rvoip_rtp_core::api::common::config::SecurityMode;
use rvoip_rtp_core::api::client::security::ClientSecurityConfig;
use rvoip_rtp_core::api::server::security::ServerSecurityConfig;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    println!("Secure Media Streaming Example");
    println!("==============================\n");

    // Configure server with DTLS-SRTP security
    println!("Creating secure server...");
    let server_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 20000);
    
    // Create security config
    let mut server_security_config = ServerSecurityConfig::default();
    server_security_config.security_mode = SecurityMode::DtlsSrtp;
    // Use generated certificate files for the server
    server_security_config.certificate_path = Some("server-cert.pem".to_string());
    server_security_config.private_key_path = Some("server-key.pem".to_string());
    
    let server_config = ServerConfigBuilder::new()
        .local_address(server_addr)
        .default_payload_type(8) // G.711 µ-law
        .clock_rate(8000)
        .security_config(server_security_config)
        .build()?;
    
    // Create server
    let server = DefaultMediaTransportServer::new(server_config).await?;
    
    // Set up server event handlers using the current API
    server.on_event(Box::new(move |event| {
        match event {
            MediaTransportEvent::Connected => {
                println!("[SERVER] New connection established");
            },
            MediaTransportEvent::Disconnected => {
                println!("[SERVER] Connection disconnected");
            },
            MediaTransportEvent::FrameReceived(frame) => {
                println!("[SERVER] Received frame: seq={}, size={} bytes", 
                    frame.sequence, frame.data.len());
            },
            MediaTransportEvent::Error(err) => {
                println!("[SERVER] Error: {}", err);
            },
            _ => {}
        }
    })).await?;
    
    // Handler for client connections
    server.on_client_connected(Box::new(|client| {
        println!("[SERVER] Client connected: {} from {}", client.id, client.address);
    })).await?;
    
    // Handler for client disconnections
    server.on_client_disconnected(Box::new(|client| {
        println!("[SERVER] Client disconnected: {} from {}", client.id, client.address);
    })).await?;
    
    // Start server
    println!("Starting server on {}...", server_addr);
    server.start().await?;
    println!("Server started successfully");
    
    // Get the actual server address after binding
    let actual_server_addr = match server.get_local_address().await {
        Ok(addr) => addr,
        Err(e) => {
            println!("[ERROR] Failed to get server address: {}", e);
            server_addr // Fallback to the configured address
        }
    };
    println!("Server actually bound to: {}", actual_server_addr);
    
    // Create test media frame generator task
    let client_task = tokio::spawn(async move {
        // Wait for server to fully initialize
        tokio::time::sleep(Duration::from_millis(500)).await;
        
        // Configure client with DTLS-SRTP security
        println!("\nCreating secure client...");
        let remote_addr = actual_server_addr;
        
        // Set up sequence number and timestamp generation
        let mut sequence: u16 = 0; // Changed to u16 to match the MediaFrame type
        let mut timestamp: u32 = 0;
        
        // Create client security config
        let mut client_security_config = ClientSecurityConfig::default();
        client_security_config.security_mode = SecurityMode::DtlsSrtp;
        // Now the client can also use certificate paths
        client_security_config.certificate_path = Some("client-cert.pem".to_string());
        client_security_config.private_key_path = Some("client-key.pem".to_string());
        
        // Configure client using the current API
        let client_config = ClientConfigBuilder::new()
            .remote_address(remote_addr)
            .default_payload_type(8) // G.711 µ-law
            .clock_rate(8000)
            .security_config(client_security_config)
            .build();
        
        // Create client
        println!("Creating client to connect to: {}", remote_addr);
        let client = DefaultMediaTransportClient::new(client_config).await?;
        
        // Set up client event handlers using the current API
        client.on_event(Box::new(|event| {
            match event {
                MediaTransportEvent::Connected => {
                    println!("[CLIENT] Connected to server");
                },
                MediaTransportEvent::Disconnected => {
                    println!("[CLIENT] Disconnected from server");
                },
                MediaTransportEvent::Error(err) => {
                    println!("[CLIENT] Error: {}", err);
                },
                _ => {}
            }
        })).await?;
        
        // Register connection callback
        client.on_connect(Box::new(|| {
            println!("[CLIENT] Connection established");
        })).await?;
        
        // Register disconnection callback
        client.on_disconnect(Box::new(|| {
            println!("[CLIENT] Connection terminated");
        })).await?;
        
        // Connect to server
        println!("Connecting to server at {}...", remote_addr);
        client.connect().await?;
        
        // Check if we're successfully connected
        if !client.is_connected().await? {
            println!("[CLIENT] Failed to connect to server!");
            return Err("Connection failed".into());
        }
        
        // Wait for security to be established
        println!("Waiting for secure connection to be established...");
        tokio::time::sleep(Duration::from_secs(2)).await;
        
        // Get security info
        if let Ok(security_info) = client.get_security_info().await {
            if let Some(fingerprint) = security_info.fingerprint {
                println!("[CLIENT] Local fingerprint: {}", fingerprint);
            }
            
            // Remote fingerprint isn't a direct field - we'd need to get this from
            // a different source or modify the SecurityInfo type if needed
            println!("[CLIENT] Security mode: {:?}", security_info.mode);
            if let Some(algo) = &security_info.fingerprint_algorithm {
                println!("[CLIENT] Fingerprint algorithm: {}", algo);
            }
            if !security_info.crypto_suites.is_empty() {
                println!("[CLIENT] Crypto suites: {:?}", security_info.crypto_suites);
            }
        }
        
        // Send test frames
        println!("\nSending 5 secure media frames...");
        for i in 0..5 {
            // Create test audio frame
            let frame = MediaFrame {
                frame_type: MediaFrameType::Audio,
                data: vec![1, 2, 3, 4, 5, 6, 7, 8],
                sequence,
                timestamp,
                payload_type: 8,
                marker: false,
                ssrc: 12345,
            };
            
            // Send frame
            client.send_frame(frame).await?;
            println!("[CLIENT] Sent frame #{}: seq={}, ts={}", i+1, sequence, timestamp);
            
            // Update sequence and timestamp
            sequence = sequence.wrapping_add(1);
            timestamp = timestamp.wrapping_add(160); // 20ms at 8kHz
            
            // Wait before sending next frame
            tokio::time::sleep(Duration::from_millis(100)).await;
        }
        
        // Wait for all frames to be processed
        tokio::time::sleep(Duration::from_secs(1)).await;
        
        // Disconnect client
        println!("\nDisconnecting client...");
        client.disconnect().await?;
        
        Ok::<(), Box<dyn std::error::Error + Send + Sync>>(())
    });
    
    // Wait for client task to complete with timeout
    tokio::select! {
        res = client_task => {
            match res {
                Ok(result) => {
                    if let Err(e) = result {
                        println!("Client task failed: {}", e);
                    }
                },
                Err(e) => {
                    println!("Client task panicked: {}", e);
                }
            }
        }
        _ = tokio::time::sleep(Duration::from_secs(15)) => {
            println!("Client task timed out after 15 seconds");
        }
    }
    
    // Stop server
    println!("\nStopping server...");
    server.stop().await?;
    println!("Server stopped successfully");
    
    println!("\nSecure media streaming example completed");
    Ok(())
} 