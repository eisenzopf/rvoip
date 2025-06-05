//! Clean Session-Core API Demo
//!
//! This example demonstrates the clean, high-level API where users only need to import
//! session-core and don't need to deal with dialog-core implementation details.

use anyhow::Result;
use tracing::{info, error};
use tokio::time::{sleep, Duration};

// Only import session-core - no dialog-core needed!
use rvoip_session_core::api::{
    ServerConfig, ClientConfig, 
    create_sip_server, create_sip_client
};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    info!("🚀 Starting Clean Session-Core API Demo");

    // === CLEAN SERVER CREATION ===
    // Users only need session-core config - no dialog-core imports!
    let server_config = ServerConfig::new("127.0.0.1:5060".parse().unwrap())
        .with_server_name("demo-server".to_string())
        .with_max_sessions(50);

    info!("📡 Creating SIP server with clean API...");
    let mut server = create_sip_server(server_config).await?;
    info!("✅ SIP server created successfully!");

    // === CLEAN CLIENT CREATION ===
    // Users only need session-core config - no dialog-core imports!
    let client_config = ClientConfig::new()
        .with_local_address("127.0.0.1:0".parse().unwrap())
        .with_from_uri("sip:client@localhost".to_string())
        .with_user_agent("demo-client".to_string());

    info!("📱 Creating SIP client with clean API...");
    let client = create_sip_client(client_config).await?;
    info!("✅ SIP client created successfully!");

    // === DEMONSTRATE CLEAN API USAGE ===
    info!("🎯 Demonstrating clean session-core API...");

    // Make a call using the clean client API
    info!("📞 Making outgoing call...");
    match client.make_call("sip:target@example.com").await {
        Ok(session_id) => {
            info!("✅ Call initiated successfully: {}", session_id);
            
            // Check active sessions
            let active_sessions = client.get_active_sessions().await;
            info!("📊 Active client sessions: {}", active_sessions.len());
            
            // Simulate call duration
            sleep(Duration::from_secs(2)).await;
            
            // Hang up the call
            if let Err(e) = client.hangup_call(&session_id).await {
                error!("❌ Failed to hang up call: {}", e);
            } else {
                info!("📴 Call hung up successfully");
            }
        }
        Err(e) => {
            error!("❌ Failed to make call: {}", e);
        }
    }

    // Demonstrate server capabilities
    let server_sessions = server.get_active_sessions().await;
    info!("📊 Active server sessions: {}", server_sessions.len());

    info!("🎉 Clean API Demo completed successfully!");
    info!("");
    info!("🔥 KEY BENEFITS:");
    info!("   ✅ Users only import session-core");
    info!("   ✅ No dialog-core imports needed");
    info!("   ✅ Clean, simple configuration");
    info!("   ✅ High-level abstractions");

    Ok(())
} 