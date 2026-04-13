//! Registration Client (UAC) Demo
//!
//! This example demonstrates client-side REGISTER functionality using session-core-v3.
//! It registers with the UAS server and handles digest authentication.
//!
//! # Architecture
//!
//! ```text
//! session-core-v3 → dialog-core → Network (REGISTER)
//!                                      ↓
//!                                   401 response
//!                                      ↓
//! session-core-v3 computes digest auth
//!                   ↓
//! dialog-core → Network (REGISTER with Authorization)
//!                   ↓
//!                200 OK
//! ```
//!
//! # Usage
//!
//! First, start the server (UAS):
//!   cargo run --example register_demo --bin uas
//!
//! Then run this client:
//!   cargo run --example register_demo --bin uac

use rvoip_session_core_v3::{
    UnifiedCoordinator,
    api::unified::Config,
    Result,
};
use std::time::Duration;
use tracing::{info, error};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_target(false)
        .with_max_level(tracing::Level::INFO)
        .init();

    info!("🚀 Starting Registration Client (UAC)");
    info!("======================================");
    
    // Create client coordinator on port 5061
    info!("\n📱 Creating client coordinator...");
    let client_config = Config {
        local_ip: "127.0.0.1".parse().unwrap(),
        sip_port: 5061,
        bind_addr: "127.0.0.1:5061".parse().unwrap(),
        local_uri: "sip:alice@127.0.0.1:5061".to_string(),
        media_port_start: 16000,
        media_port_end: 17000,
        state_table_path: None,
    };

    let coordinator = UnifiedCoordinator::new(client_config).await?;
    info!("✅ Client coordinator created on 127.0.0.1:5061");
    
    // Give everything a moment to settle
    tokio::time::sleep(Duration::from_millis(500)).await;
    
    // Registration parameters
    info!("\n🔐 Starting registration...");
    let registrar_uri = "sip:127.0.0.1:5060";
    let from_uri = "sip:alice@127.0.0.1";
    let contact_uri = "sip:alice@127.0.0.1:5061";
    let username = "alice";
    let password = "password123";
    let expires = 3600;  // 1 hour

    info!("Registration parameters:");
    info!("  Registrar: {}", registrar_uri);
    info!("  From: {}", from_uri);
    info!("  Contact: {}", contact_uri);
    info!("  Username: {}", username);
    info!("  Expires: {} seconds", expires);
    info!("");

    // Register with server
    match coordinator.register(
        registrar_uri,
        from_uri,
        contact_uri,
        username,
        password,
        expires,
    ).await {
        Ok(handle) => {
            info!("✅ Registration initiated!");
            info!("   Session ID: {}", handle.session_id.0);

            // Wait for registration to complete (auth flow takes ~1-2 seconds)
            info!("\n⏳ Waiting for registration to complete...");
            tokio::time::sleep(Duration::from_secs(5)).await;

            // Check registration status
            match coordinator.is_registered(&handle).await {
                Ok(true) => {
                    info!("\n✅ Registration successful!");
                    info!("   User alice is now registered with the server");
                    
                    // Keep alive for a while with periodic refresh
                    info!("\n🔄 Keeping registration alive...");
                    
                    for i in 1..=3 {
                        tokio::time::sleep(Duration::from_secs(10)).await;
                        
                        info!("\n🔄 Refreshing registration (attempt {}/3)...", i);
                        match coordinator.refresh_registration(&handle).await {
                            Ok(_) => info!("✅ Registration refreshed successfully"),
                            Err(e) => error!("❌ Failed to refresh registration: {}", e),
                        }
                    }
                    
                    // Unregister
                    info!("\n📤 Unregistering...");
                    match coordinator.unregister(&handle).await {
                        Ok(_) => info!("✅ Unregistered successfully"),
                        Err(e) => error!("❌ Failed to unregister: {}", e),
                    }
                    
                    info!("\n✅ Demo completed successfully!");
                }
                Ok(false) => {
                    error!("\n❌ Registration not completed");
                    error!("   Make sure the server (UAS) is running:");
                    error!("   cargo run --example register_demo --bin uas");
                }
                Err(e) => {
                    error!("\n❌ Failed to check registration status: {}", e);
                }
            }
        }
        Err(e) => {
            error!("\n❌ Registration failed: {}", e);
            error!("   Make sure the server (UAS) is running:");
            error!("   cargo run --example register_demo --bin uas");
        }
    }

    Ok(())
}

