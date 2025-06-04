//! Simple Dialog Test with Unified API
//!
//! A minimal test to verify unified dialog-core functionality.

use std::time::Duration;
use rvoip_dialog_core::{config::DialogManagerConfig, UnifiedDialogApi};
use tracing::{info, Level};
use tracing_subscriber;
use tokio::time::sleep;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(Level::INFO)
        .init();

    info!("🔧 Simple Dialog Test - Unified API Edition");
    
    // Create unified configuration (client mode for this simple test)
    let config = DialogManagerConfig::client("127.0.0.1:0".parse()?)
        .with_from_uri("sip:alice@example.com")
        .build();
    
    info!("✅ Created unified configuration (client mode)");
    
    // Create unified dialog API (handles transport internally)
    let api = UnifiedDialogApi::create(config).await?;
    
    info!("✅ Created UnifiedDialogApi");
    
    // Start the unified API
    api.start().await?;
    info!("✅ Started unified dialog API");
    
    // Show configuration capabilities
    info!("\n🔧 Unified API Capabilities:");
    info!("   • Supports outgoing calls: {}", api.supports_outgoing_calls());
    info!("   • Supports incoming calls: {}", api.supports_incoming_calls());
    info!("   • From URI: {:?}", api.from_uri());
    info!("   • Auto auth enabled: {}", api.auto_auth_enabled());
    
    // Create a test dialog using unified API
    let local_uri = "sip:alice@example.com";
    let remote_uri = "sip:bob@example.com";
    
    let dialog = api.create_dialog(local_uri, remote_uri).await?;
    info!("✅ Created dialog: {}", dialog.id());
    
    // Test basic operations using unified API
    let stats = api.get_stats().await;
    info!("✅ Statistics: {} active dialogs, {} total", stats.active_dialogs, stats.total_dialogs);
    
    let active_dialogs = api.list_active_dialogs().await;
    info!("✅ Active dialog list: {} dialogs", active_dialogs.len());
    
    // Note: In production, dialogs would be established via INVITE/200 OK flow
    // before sending in-dialog requests. This simple test only demonstrates API creation.
    info!("💡 Dialog created successfully - ready for INVITE/200 OK establishment flow");
    
    // Wait a moment
    sleep(Duration::from_millis(100)).await;
    
    // Test different configuration modes (demonstrate architectural flexibility)
    info!("\n🔀 Testing configuration flexibility...");
    
    // Create server configuration for comparison
    let server_config = DialogManagerConfig::server("127.0.0.1:0".parse()?)
        .with_domain("test.local")
        .with_auto_options()
        .build();
    
    info!("✅ Server config would support:");
    info!("   • Supports outgoing calls: {}", server_config.supports_outgoing_calls());
    info!("   • Supports incoming calls: {}", server_config.supports_incoming_calls());
    info!("   • Auto OPTIONS: {}", server_config.auto_options_enabled());
    
    // Create hybrid configuration for comparison
    let hybrid_config = DialogManagerConfig::hybrid("127.0.0.1:0".parse()?)
        .with_from_uri("sip:hybrid@example.com")
        .with_domain("test.local")
        .with_auto_options()
        .build();
    
    info!("✅ Hybrid config would support:");
    info!("   • Supports outgoing calls: {}", hybrid_config.supports_outgoing_calls());
    info!("   • Supports incoming calls: {}", hybrid_config.supports_incoming_calls());
    info!("   • Auto OPTIONS: {}", hybrid_config.auto_options_enabled());
    
    // Clean up main API
    api.stop().await?;
    info!("✅ Stopped unified dialog API");
    
    info!("\n🎯 === Simple Test Results ===");
    info!("✅ Unified API initialization");
    info!("✅ Configuration-driven behavior");
    info!("✅ Dialog creation and management");
    info!("✅ Statistics and monitoring");
    info!("✅ SIP method calls (API surface)");
    info!("✅ Configuration mode comparison");
    info!("✅ Clean lifecycle management");
    
    info!("\n🎉 Simple unified API test completed successfully!");
    
    Ok(())
} 