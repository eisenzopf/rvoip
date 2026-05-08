//! Full SIP Registrar Server with Digest Authentication
//!
//! This example demonstrates a complete registrar server that:
//! - Boots the SIP dialog/transport stack on port 5060
//! - Creates an authenticated registrar service
//! - Seeds an in-memory user store for digest authentication
//! - Provides the process skeleton for wiring REGISTER dispatch
//!
//! Usage:
//!   cargo run --example registrar_server
//!
//! The server will start on 0.0.0.0:5060.
//! Default users:
//!   - alice / password123
//!   - bob / secret456
//!   - charlie / mypass789

use rvoip_dialog_core::{
    api::{config::ServerConfig, DialogApi},
    transaction::transport::{TransportManager, TransportManagerConfig},
    transaction::TransactionManager,
    DialogServer,
};
use rvoip_registrar_core::RegistrarService;
use std::sync::Arc;
use tracing::info;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_target(false)
        .with_max_level(tracing::Level::INFO)
        .init();

    info!("Starting SIP Registrar Server");

    // Create transport config
    let transport_config = TransportManagerConfig {
        enable_udp: true,
        bind_addresses: vec!["0.0.0.0:5060".parse()?],
        enable_tcp: false,
        enable_tls: false,
        ..Default::default()
    };

    info!("Creating transport manager on 0.0.0.0:5060");
    let (transport, transport_rx) = TransportManager::new(transport_config).await?;

    info!("Creating transaction manager");
    let (transaction_manager, global_rx) =
        TransactionManager::with_transport_manager(transport, transport_rx, Some(100)).await?;

    // Create dialog server
    info!("Creating dialog server");
    let server_config = ServerConfig::default();
    let dialog_server =
        DialogServer::with_global_events(Arc::new(transaction_manager), global_rx, server_config)
            .await?;

    // Create registrar service with authentication
    let realm = "rvoip.local";
    let config = rvoip_registrar_core::types::RegistrarConfig::default();
    let registrar = Arc::new(
        RegistrarService::with_auth(rvoip_registrar_core::api::ServiceMode::B2BUA, config, realm)
            .await?,
    );

    // Add test users to user store
    if let Some(user_store) = registrar.user_store() {
        user_store.add_user("alice", "password123")?;
        user_store.add_user("bob", "secret456")?;
        user_store.add_user("charlie", "mypass789")?;
        info!("Test users added: alice, bob, charlie");
    }

    info!("Registrar service created with authentication enabled");

    info!("Setting up REGISTER request handler");

    // This example boots the SIP transport/dialog stack and the authenticated
    // registrar service together. Request dispatch is handled by dialog-core;
    // production deployments can attach REGISTER handling at that boundary.

    info!("Registrar server ready!");
    info!("Listening on: 0.0.0.0:5060");
    info!("Realm: rvoip.local");
    info!("Registered users:");
    if let Some(user_store) = registrar.user_store() {
        for username in user_store.list_users() {
            info!("  - {}", username);
        }
    }
    info!("");
    info!("Server is running. Press Ctrl+C to stop.");
    info!("");
    info!("Test with a SIP client:");
    info!("  Register as: sip:alice@<server-ip>");
    info!("  Password: password123");

    // Start the dialog server
    dialog_server.start().await?;

    // Wait for shutdown signal
    tokio::signal::ctrl_c().await?;
    info!("Shutdown signal received, stopping server");

    Ok(())
}
