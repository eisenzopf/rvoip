/// Port Allocation Demo
///
/// This example demonstrates how to get dynamically allocated port information
/// from both client and server transport APIs. This shows how session-core
/// would obtain this information for SDP signaling.

use std::net::{SocketAddr, IpAddr, Ipv4Addr};
use std::sync::Arc;
use std::time::Duration;

use rvoip_rtp_core::api::server::transport::{MediaTransportServer, DefaultMediaTransportServer};
use rvoip_rtp_core::api::client::transport::{MediaTransportClient, DefaultMediaTransportClient};
use rvoip_rtp_core::api::server::config::{ServerConfig, ServerConfigBuilder};
use rvoip_rtp_core::api::client::config::{ClientConfig, ClientConfigBuilder};
use rvoip_rtp_core::api::common::config::SecurityMode;
use rvoip_rtp_core::api::client::security::ClientSecurityConfig;
use rvoip_rtp_core::api::server::security::ServerSecurityConfig;
use tokio::time::timeout;

async fn run_with_timeout() -> Result<(), Box<dyn std::error::Error>> {
    println!("Port Allocation Demo");
    println!("===================\n");
    
    // Set up a server using dynamic port allocation
    let server_addr = SocketAddr::new(IpAddr::V4(Ipv4Addr::new(127, 0, 0, 1)), 0); // Port 0 means use dynamic allocation
    
    // Create security config with no security
    let mut server_security_config = ServerSecurityConfig::default();
    server_security_config.security_mode = SecurityMode::None;
    
    let server_config = ServerConfigBuilder::new()
        .local_address(server_addr)
        .default_payload_type(8) // G.711 µ-law
        .clock_rate(8000)
        .security_config(server_security_config)
        .build()?;
    
    // Create and start server
    println!("[SERVER] Creating server...");
    let server = DefaultMediaTransportServer::new(server_config).await?;
    
    println!("[SERVER] Starting server...");
    match timeout(Duration::from_secs(2), server.start()).await {
        Ok(result) => result?,
        Err(_) => {
            println!("[ERROR] Server start timed out after 2 seconds");
            return Err("Server start timeout".into());
        }
    }
    
    // Get the actual port allocated to the server
    // This is what session-core would do after starting the server
    let actual_server_addr = match timeout(Duration::from_secs(1), server.get_local_address()).await {
        Ok(result) => result?,
        Err(_) => {
            println!("[ERROR] Getting server address timed out");
            return Err("Server address timeout".into());
        }
    };
    println!("[SERVER] Server bound to: {}", actual_server_addr);
    
    // Set up a client also using dynamic port allocation
    let client_remote_addr = actual_server_addr; // Use the actual server address obtained above
    
    // Create security config with no security
    let mut client_security_config = ClientSecurityConfig::default();
    client_security_config.security_mode = SecurityMode::None;
    
    let client_config = ClientConfigBuilder::new()
        .remote_address(client_remote_addr)
        .default_payload_type(8) // G.711 µ-law
        .clock_rate(8000)
        .security_config(client_security_config)
        .build();
    
    // Create client
    println!("\n[CLIENT] Creating client...");
    let client = DefaultMediaTransportClient::new(client_config).await?;
    
    // Connect client to server (this will allocate client ports)
    println!("[CLIENT] Connecting to server...");
    match timeout(Duration::from_secs(2), client.connect()).await {
        Ok(result) => result?,
        Err(_) => {
            println!("[ERROR] Client connect timed out after 2 seconds");
            return Err("Client connect timeout".into());
        }
    }
    
    // Get the actual port allocated to the client
    // This is what session-core would do after connecting
    let actual_client_addr = match timeout(Duration::from_secs(1), client.get_local_address()).await {
        Ok(result) => result?,
        Err(_) => {
            println!("[ERROR] Getting client address timed out");
            return Err("Client address timeout".into());
        }
    };
    println!("[CLIENT] Client bound to: {}", actual_client_addr);
    
    // Demonstrate that these would be exchanged in SDP
    println!("\n[SDP] Server would advertise its RTP address: {}", actual_server_addr);
    println!("[SDP] Client would advertise its RTP address: {}", actual_client_addr);
    
    // Clean up
    println!("\n[CLEANUP] Disconnecting client...");
    if let Err(e) = timeout(Duration::from_secs(1), client.disconnect()).await {
        println!("[WARNING] Client disconnect timed out: {}", e);
    }
    
    // Allow disconnection to complete
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    println!("[CLEANUP] Stopping server...");
    if let Err(e) = timeout(Duration::from_secs(1), server.stop()).await {
        println!("[WARNING] Server stop timed out: {}", e);
    }
    
    println!("\nPort Allocation Demo completed successfully");
    Ok(())
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Run the demo with an overall timeout of 10 seconds
    match timeout(Duration::from_secs(10), run_with_timeout()).await {
        Ok(result) => result,
        Err(_) => {
            println!("[FATAL] The entire demo timed out after 10 seconds");
            Err("Demo timeout".into())
        }
    }
} 