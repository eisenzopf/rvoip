//! Registration Server (UAS) Demo
//!
//! This example demonstrates server-side REGISTER handling using session-core-v3
//! to orchestrate between dialog-core (protocol) and registrar-core (authentication/storage).
//!
//! # Architecture
//!
//! ```text
//! Network → dialog-core → IncomingRegister event → session-core-v3 → registrar-core
//!                                                         ↓
//!                                                   authenticate_register()
//!                                                         ↓
//!          dialog-core ← SendRegisterResponse event ← session-core-v3
//!                ↓
//!          Sends 401/200 SIP response
//! ```
//!
//! # Usage
//!
//! Start this server first:
//!   cargo run --example register_demo --bin uas
//!
//! Then run the client (UAC) in another terminal:
//!   cargo run --example register_demo --bin uac

use rvoip_session_core_v3::{
    UnifiedCoordinator,
    api::unified::Config,
    Result,
};
use std::collections::HashMap;
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_target(false)
        .with_max_level(tracing::Level::INFO)
        .init();

    info!("🚀 Starting Registration Server (UAS)");
    info!("=====================================");
    
    // Create server coordinator on port 5060
    info!("\n📡 Creating server coordinator...");
    let server_config = Config {
        local_ip: "127.0.0.1".parse().unwrap(),
        sip_port: 5060,
        bind_addr: "127.0.0.1:5060".parse().unwrap(),
        local_uri: "sip:registrar@127.0.0.1:5060".to_string(),
        media_port_start: 15000,
        media_port_end: 16000,
        state_table_path: None,
    };
    
    let server_coordinator = UnifiedCoordinator::new(server_config).await?;
    info!("✅ Server coordinator created on 127.0.0.1:5060");
    
    // Start server-side registration handling with test users
    info!("\n🔐 Starting registration server...");
    info!("Realm: test.local");
    
    let mut users = HashMap::new();
    users.insert("alice".to_string(), "password123".to_string());
    users.insert("bob".to_string(), "secret456".to_string());
    
    let _registrar = server_coordinator.start_registration_server("test.local", users).await?;
    
    info!("✅ Registrar server started");
    info!("\n👥 Registered users:");
    info!("  - alice / password123");
    info!("  - bob / secret456");
    info!("\n📞 Server ready to accept REGISTER requests on 127.0.0.1:5060");
    info!("\n🔄 Expected flow:");
    info!("  1. Client sends REGISTER (no auth)");
    info!("  2. Server responds 401 with challenge");
    info!("  3. Client sends REGISTER with digest auth");
    info!("  4. Server validates and responds 200 OK");
    info!("  5. Registration stored in registrar-core");
    info!("\n💡 Run the client: cargo run --example register_demo --bin uac");
    info!("\nPress Ctrl+C to stop the server...\n");
    
    // Keep the server running
    tokio::signal::ctrl_c().await
        .expect("Failed to listen for ctrl-c");
    
    info!("\n🛑 Shutting down server...");
    Ok(())
}

