//! Basic Dialog Management Example
//!
//! This example demonstrates proper SIP dialog patterns with the unified
//! DialogManager API, focusing on correct usage and best practices.

use std::time::Duration;
use rvoip_dialog_core::{UnifiedDialogApi, config::DialogManagerConfig};
use tracing::{info, warn, Level};
use tracing_subscriber;
use tokio::time::sleep;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .init();

    info!("🚀 Starting Basic Dialog Example - Proper SIP Patterns");

    // Create unified configuration in client mode (dialog-core handles transport internally)
    let config = DialogManagerConfig::client("127.0.0.1:0".parse()?)
        .with_from_uri("sip:alice@example.com")
        .build();
        
    let api = UnifiedDialogApi::create(config).await?;
    
    info!("✅ Initialized unified dialog API in client mode");

    // Start the API
    api.start().await?;

    // Create dialogs with proper URIs
    let local_uri = "sip:alice@example.com";
    let remote_uri = "sip:bob@example.com";
    
    let dialog = api.create_dialog(local_uri, remote_uri).await?;
    info!("✅ Created dialog: {}", dialog.id());
    
    // Demonstrate real SIP call establishment (proper way)
    info!("\n📞 === Real SIP Call Establishment ===");
    
    // Make a real call - this creates a dialog and sends INVITE
    let call_result = api.make_call(local_uri, remote_uri, None).await;
    match call_result {
        Ok(call) => {
            info!("✅ Successfully initiated real SIP call: {}", call.call_id());
            info!("📋 Real dialog created through INVITE request");
        },
        Err(e) => {
            info!("⚠️  Call failed (expected in test environment): {}", e);
            info!("💡 In production, this would establish a real dialog via INVITE/200 OK");
        }
    }
    
    // Display statistics
    let stats = api.get_stats().await;
    info!("\n📊 === API Statistics ===");
    info!("Active dialogs: {}", stats.active_dialogs);
    info!("Total dialogs: {}", stats.total_dialogs);
    
    // Demonstrate proper dialog termination
    info!("\n🔚 === Proper Dialog Termination ===");
    let terminate_result = api.terminate_dialog(dialog.id()).await;
    match terminate_result {
        Ok(_) => info!("✅ Dialog terminated properly"),
        Err(e) => warn!("❌ Termination error: {}", e),
    }
    
    // Final statistics
    let final_stats = api.get_stats().await;
    info!("Final active dialogs: {}", final_stats.active_dialogs);
    
    info!("\n🎯 === Best Practices Demonstrated ===");
    info!("✅ Create dialog with proper SIP URIs");
    info!("✅ Use make_call() for real dialog establishment via INVITE");
    info!("✅ Only send in-dialog requests to confirmed dialogs (not shown here)");
    info!("✅ Properly terminate dialogs when done");
    info!("💡 This example shows real SIP patterns - no simulated establishment");

    // Stop the API
    api.stop().await?;

    // Brief pause before shutdown
    sleep(Duration::from_millis(100)).await;
    
    info!("🏁 Basic dialog example completed successfully");
    Ok(())
} 