//! Debug State Error Example
//!
//! A minimal reproduction of the StateChanged event failure that occurs when
//! dialog-core tries to send SIP requests using Phase 3 helper functions.

use std::sync::Arc;
use tokio::time::{sleep, Duration};
use tracing::{info, error, Level};
use tracing_subscriber;

use rvoip_dialog_core::api::{DialogClient, DialogApi};
use rvoip_transaction_core::TransactionManager;
use rvoip_transaction_core::transport::{TransportManager, TransportManagerConfig};
use rvoip_sip_core::Uri;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(Level::DEBUG)
        .init();

    info!("🔧 Debug State Error - Minimal Reproduction");
    
    // Set up a simple client with minimal configuration
    let client_config = TransportManagerConfig {
        enable_udp: true,
        enable_tcp: false,
        enable_ws: false,
        enable_tls: false,
        bind_addresses: vec!["127.0.0.1:0".parse()?],
        ..Default::default()
    };
    
    let (mut client_transport, client_transport_rx) = TransportManager::new(client_config).await?;
    client_transport.initialize().await?;
    let client_addr = client_transport.default_transport().await
        .ok_or("No default transport")?.local_addr()?;
    
    let (client_transaction_manager, _) = TransactionManager::with_transport_manager(
        client_transport,
        client_transport_rx,
        Some(100),
    ).await?;
    
    let client_config = rvoip_dialog_core::api::config::ClientConfig::default();
    
    let client = DialogClient::with_dependencies(
        Arc::new(client_transaction_manager),
        client_config
    ).await?;
    
    info!("✅ Client bound to: {}", client_addr);
    
    // Start the client
    client.start().await?;
    info!("✅ Started dialog client");
    
    // Create a dialog
    let local_uri: Uri = format!("sip:alice@{}", client_addr).parse()?;
    let remote_uri: Uri = "sip:bob@127.0.0.1:5060".parse()?; // Non-existent endpoint
    
    let dialog_id = client.create_outgoing_dialog(local_uri, remote_uri, None).await?;
    info!("✅ Created dialog: {}", dialog_id);
    
    // This is where the error occurs - sending INFO request
    info!("🔥 Attempting to send INFO request (this should fail with StateChanged event error)...");
    
    match client.send_info(&dialog_id, "Test info content".to_string()).await {
        Ok(transaction_id) => {
            info!("✅ INFO request sent successfully - Transaction: {}", transaction_id);
        },
        Err(e) => {
            error!("❌ Failed to send INFO request: {}", e);
            info!("💡 This is the root cause of the Phase 3 integration showcase failure");
        }
    }
    
    // Try a few more methods to see if the issue is consistent
    info!("🔥 Attempting to send UPDATE request...");
    match client.send_update(&dialog_id, Some("v=0\r\no=test 123 456 IN IP4 127.0.0.1\r\n".to_string())).await {
        Ok(transaction_id) => {
            info!("✅ UPDATE request sent successfully - Transaction: {}", transaction_id);
        },
        Err(e) => {
            error!("❌ Failed to send UPDATE request: {}", e);
        }
    }
    
    info!("🔥 Attempting to send NOTIFY request...");
    match client.send_notify(&dialog_id, "test-event".to_string(), Some("Test notification".to_string())).await {
        Ok(transaction_id) => {
            info!("✅ NOTIFY request sent successfully - Transaction: {}", transaction_id);
        },
        Err(e) => {
            error!("❌ Failed to send NOTIFY request: {}", e);
        }
    }
    
    // Give time for any async operations to complete
    sleep(Duration::from_millis(500)).await;
    
    // Clean up
    client.stop().await?;
    info!("✅ Client stopped");
    
    info!("🎯 Debug analysis complete. The issue appears to be with the StateChanged event system in transaction-core when called through dialog-core.");
    
    Ok(())
} 