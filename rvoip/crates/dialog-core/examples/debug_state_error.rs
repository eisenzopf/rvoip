//! Debug State Error Example
//!
//! This example demonstrates proper SIP dialog state validation and shows
//! the difference between unestablished and established dialogs.

use std::sync::Arc;
use tokio::time::{sleep, Duration};
use tracing::{info, error, Level};
use tracing_subscriber;

use rvoip_dialog_core::api::{DialogClient, DialogApi};
use rvoip_dialog_core::{DialogState};
use rvoip_transaction_core::TransactionManager;
use rvoip_transaction_core::transport::{TransportManager, TransportManagerConfig};
use rvoip_sip_core::Uri;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(Level::DEBUG)
        .init();

    info!("🔧 SIP Dialog State Validation Example");
    info!("   Demonstrates proper dialog establishment vs. unestablished dialog errors");
    info!("   Using GLOBAL EVENTS pattern for reliable transaction handling");
    
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
    
    // Use GLOBAL EVENTS pattern for reliable event handling
    let (client_transaction_manager, client_global_rx) = TransactionManager::with_transport_manager(
        client_transport,
        client_transport_rx,
        Some(100),
    ).await?;
    
    let client_config = rvoip_dialog_core::api::config::ClientConfig::default();
    
    // Create client with GLOBAL EVENTS (recommended pattern)
    let client = DialogClient::with_global_events(
        Arc::new(client_transaction_manager),
        client_global_rx,
        client_config
    ).await?;
    
    info!("✅ Client bound to: {}", client_addr);
    
    // Start the client
    client.start().await?;
    info!("✅ Started dialog client");
    
    // === PART 1: Demonstrate EXPECTED errors for unestablished dialogs ===
    info!("\n🔥 === Part 1: Unestablished Dialog (EXPECTED ERRORS) ===");
    
    let local_uri: Uri = format!("sip:alice@{}", client_addr).parse()?;
    let remote_uri: Uri = "sip:bob@127.0.0.1:5060".parse()?;
    
    let dialog = client.create_dialog(&local_uri.to_string(), &remote_uri.to_string()).await?;
    let dialog_id = dialog.id().clone();
    info!("✅ Created unestablished dialog: {}", dialog_id);
    
    let state = client.get_dialog_state(&dialog_id).await?;
    info!("📋 Dialog state: {:?} (no remote tag yet)", state);
    
    info!("🔥 Attempting to send INFO request on unestablished dialog (SHOULD FAIL)...");
    match client.send_info(&dialog_id, "Test info content".to_string()).await {
        Ok(_) => {
            error!("❌ UNEXPECTED: INFO request succeeded on unestablished dialog!");
        },
        Err(e) => {
            info!("✅ EXPECTED: INFO request correctly rejected: {}", e);
        }
    }
    
    info!("🔥 Attempting to send UPDATE request on unestablished dialog (SHOULD FAIL)...");
    match client.send_update(&dialog_id, Some("v=0\r\no=test 123 456 IN IP4 127.0.0.1\r\n".to_string())).await {
        Ok(_) => {
            error!("❌ UNEXPECTED: UPDATE request succeeded on unestablished dialog!");
        },
        Err(e) => {
            info!("✅ EXPECTED: UPDATE request correctly rejected: {}", e);
        }
    }
    
    info!("🔥 Attempting to send NOTIFY request on unestablished dialog (SHOULD FAIL)...");
    match client.send_notify(&dialog_id, "test-event".to_string(), Some("Test notification".to_string())).await {
        Ok(_) => {
            error!("❌ UNEXPECTED: NOTIFY request succeeded on unestablished dialog!");
        },
        Err(e) => {
            info!("✅ EXPECTED: NOTIFY request correctly rejected: {}", e);
        }
    }
    
    info!("✅ SIP protocol validation working correctly - unestablished dialogs properly rejected!");
    
    // === PART 2: Demonstrate correct usage with established dialog ===
    info!("\n🚀 === Part 2: Established Dialog (SHOULD WORK) ===");
    
    // Create another dialog for establishment demo
    let dialog2 = client.create_dialog(&local_uri.to_string(), &remote_uri.to_string()).await?;
    let dialog_id2 = dialog2.id().clone();
    
    // Manually establish the dialog for testing (in production, this happens via SIP message flow)
    info!("🔧 Manually establishing dialog for testing...");
    let dialog_manager = client.dialog_manager().clone();
    {
        let mut dialog_guard = dialog_manager.get_dialog_mut(&dialog_id2)?;
        dialog_guard.remote_tag = Some("test-remote-tag".to_string());
        dialog_guard.state = DialogState::Confirmed;
    }
    
    let state2 = client.get_dialog_state(&dialog_id2).await?;
    info!("📋 Dialog state: {:?} (now has remote tag)", state2);
    
    info!("🚀 Attempting to send INFO request on established dialog (SHOULD WORK)...");
    match client.send_info(&dialog_id2, "Test info content".to_string()).await {
        Ok(transaction_id) => {
            info!("✅ SUCCESS: INFO request sent - Transaction: {}", transaction_id);
        },
        Err(e) => {
            error!("❌ UNEXPECTED: INFO request failed on established dialog: {}", e);
        }
    }
    
    info!("🚀 Attempting to send UPDATE request on established dialog (SHOULD WORK)...");
    match client.send_update(&dialog_id2, Some("v=0\r\no=test 123 456 IN IP4 127.0.0.1\r\n".to_string())).await {
        Ok(transaction_id) => {
            info!("✅ SUCCESS: UPDATE request sent - Transaction: {}", transaction_id);
        },
        Err(e) => {
            error!("❌ UNEXPECTED: UPDATE request failed on established dialog: {}", e);
        }
    }
    
    info!("🚀 Attempting to send NOTIFY request on established dialog (SHOULD WORK)...");
    match client.send_notify(&dialog_id2, "test-event".to_string(), Some("Test notification".to_string())).await {
        Ok(transaction_id) => {
            info!("✅ SUCCESS: NOTIFY request sent - Transaction: {}", transaction_id);
        },
        Err(e) => {
            error!("❌ UNEXPECTED: NOTIFY request failed on established dialog: {}", e);
        }
    }
    
    // Give time for any async operations to complete
    sleep(Duration::from_millis(500)).await;
    
    // Clean up
    client.stop().await?;
    info!("✅ Client stopped");
    
    info!("\n🎯 === Summary ===");
    info!("✅ SIP protocol validation working correctly:");
    info!("   • Unestablished dialogs properly reject in-dialog requests");
    info!("   • Established dialogs accept in-dialog requests");
    info!("✅ Global events pattern prevents StateChanged event failures");
    info!("💡 In production, dialogs are established through proper INVITE/200 OK flows");
    
    Ok(())
} 