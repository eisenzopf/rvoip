//! Simple Example Demonstrating RTP Header Extensions
//!
//! This example shows how to configure and use RTP header extensions.

use std::net::SocketAddr;
use std::time::Duration;
use std::collections::HashMap;

use rvoip_rtp_core::api::{
    server::{ServerConfigBuilder, ServerFactory, transport::MediaTransportServer},
    common::extension::ExtensionFormat,
};

use tracing::{info, debug, error};
use tokio::time;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::DEBUG)
        .init();
    
    info!("RTP Header Extensions API Example (Simplified)");
    info!("==============================================");
    
    // Configure and create server
    let server_config = ServerConfigBuilder::new()
        .local_address("127.0.0.1:9000".parse()?)
        .header_extensions_enabled(true)  // Enable header extensions
        .header_extension_format(ExtensionFormat::OneByte)  // Use one-byte format
        .build()?;
    
    // Use ServerFactory's static method to create a server
    let server = ServerFactory::create_server(server_config).await?;
    
    // Start the server
    server.start().await?;
    info!("Server started on {}", server.get_local_address().await?);
    
    // Set up a single header extension mapping (ID to URI)
    server.configure_header_extension(1, "urn:ietf:params:rtp-hdrext:ssrc-audio-level".to_string()).await?;
    
    info!("Configured audio level header extension on server");
    
    // Verify header extensions are enabled
    let extensions_enabled = server.is_header_extensions_enabled().await?;
    info!("Header extensions enabled: {}", extensions_enabled);
    
    // Wait a moment
    info!("Server running, press Ctrl+C to exit...");
    time::sleep(Duration::from_secs(5)).await;
    
    // Clean up
    info!("Stopping server...");
    server.stop().await?;
    
    info!("Test completed.");
    Ok(())
} 