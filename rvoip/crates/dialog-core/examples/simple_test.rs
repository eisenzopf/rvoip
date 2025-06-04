//! Simple Dialog Test
//!
//! A minimal test to verify dialog-core functionality without global events.

use std::sync::Arc;
use tokio::time::{sleep, Duration};
use tracing::{info, Level};
use tracing_subscriber;

use rvoip_dialog_core::manager::DialogManager;
use rvoip_transaction_core::TransactionManager;
use rvoip_transaction_core::transport::{TransportManager, TransportManagerConfig};
use rvoip_sip_core::Uri;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .init();

    info!("🔧 Simple Dialog Test - Testing without global events");
    
    // Create transport manager
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
    
    info!("✅ Client bound to: {}", client_addr);
    
    // Create TransactionManager (without using global events)
    let (transaction_manager, _transaction_events) = TransactionManager::with_transport_manager(
        client_transport.clone(),
        client_transport_rx,
        Some(100),
    ).await?;
    
    info!("✅ Created TransactionManager");
    
    // Create DialogManager using the old pattern (should work for basic operations)
    let dialog_manager = DialogManager::new(
        Arc::new(transaction_manager),
        "127.0.0.1:5060".parse()?,
    ).await?;
    
    info!("✅ Created DialogManager without global events");
    
    // Start the dialog manager
    dialog_manager.start().await?;
    info!("✅ Started dialog manager");
    
    // Create a test dialog
    let local_uri: Uri = format!("sip:alice@{}", client_addr.ip()).parse()?;
    let remote_uri: Uri = format!("sip:bob@127.0.0.1:5060").parse()?;
    let dialog_id = dialog_manager.create_outgoing_dialog(local_uri, remote_uri, None).await?;
    info!("✅ Created dialog: {}", dialog_id);
    
    // Test basic dialog operations
    let dialog_count = dialog_manager.dialog_count();
    info!("✅ Dialog count: {}", dialog_count);
    
    let has_dialog = dialog_manager.has_dialog(&dialog_id);
    info!("✅ Dialog exists: {}", has_dialog);
    
    // Wait a moment
    sleep(Duration::from_millis(100)).await;
    
    // Clean up
    dialog_manager.stop().await?;
    info!("✅ Stopped dialog manager");
    
    info!("🎯 Simple test completed successfully!");
    
    Ok(())
} 