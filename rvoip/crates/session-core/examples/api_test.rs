//! API Test Example
//!
//! This example tests the new session-core API factory functions to ensure
//! they work correctly without requiring external imports.

use rvoip_session_core::api::{
    factory::{create_sip_server, create_sip_client},
    server::config::{ServerConfig, TransportProtocol},
    client::config::ClientConfig,
};
use std::net::SocketAddr;
use tokio::time::{timeout, Duration};
use tracing::{info, error};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::fmt::init();
    
    println!("Starting session-core API tests...");
    info!("Testing session-core API factory functions");
    
    // Test server creation
    println!("Testing server creation...");
    match test_server_creation().await {
        Ok(_) => println!("✅ Server creation test passed"),
        Err(e) => {
            println!("❌ Server creation test failed: {}", e);
            return Err(e);
        }
    }
    
    // Test client creation
    println!("Testing client creation...");
    match test_client_creation().await {
        Ok(_) => println!("✅ Client creation test passed"),
        Err(e) => {
            println!("❌ Client creation test failed: {}", e);
            return Err(e);
        }
    }
    
    println!("🎉 All API tests completed successfully!");
    info!("All API tests completed successfully!");
    Ok(())
}

async fn test_server_creation() -> Result<(), Box<dyn std::error::Error>> {
    info!("Testing SIP server creation...");
    
    // Create server configuration
    let config = ServerConfig::new("127.0.0.1:5060".parse()?) // Use a proper port
        .with_transport(TransportProtocol::Udp)
        .with_max_sessions(100)
        .with_server_name("test-server".to_string());
    
    // Validate configuration
    config.validate()?;
    info!("Server configuration validated successfully");
    
    // Create server
    let server = create_sip_server(config).await?;
    info!("SIP server created successfully");
    
    // Get session manager
    let session_manager = server.session_manager();
    info!("Session manager obtained: active sessions = {}", session_manager.list_sessions().len());
    
    // Test server configuration access
    let server_config = server.config();
    info!("Server bound to: {}", server_config.bind_address);
    info!("Transport protocol: {}", server_config.transport_protocol);
    info!("Max sessions: {}", server_config.max_sessions);
    
    Ok(())
}

async fn test_client_creation() -> Result<(), Box<dyn std::error::Error>> {
    info!("Testing SIP client creation...");
    
    // Create client configuration
    let config = ClientConfig::new()
        .with_transport(TransportProtocol::Udp)
        .with_max_sessions(10)
        .with_user_agent("test-client".to_string())
        .with_credentials("testuser".to_string(), "testpass".to_string());
    
    // Validate configuration
    config.validate()?;
    info!("Client configuration validated successfully");
    
    // Create client
    let client = create_sip_client(config).await?;
    info!("SIP client created successfully");
    
    // Get session manager
    let session_manager = client.session_manager();
    info!("Session manager obtained: active sessions = {}", session_manager.list_sessions().len());
    
    // Test client configuration access
    let client_config = client.config();
    info!("Client user agent: {}", client_config.user_agent);
    info!("Client max sessions: {}", client_config.max_sessions);
    info!("Client transport: {}", client_config.transport_protocol);
    
    // Test effective URIs
    info!("Effective contact URI: {}", client_config.effective_contact_uri());
    info!("Effective from URI: {}", client_config.effective_from_uri());
    
    Ok(())
} 