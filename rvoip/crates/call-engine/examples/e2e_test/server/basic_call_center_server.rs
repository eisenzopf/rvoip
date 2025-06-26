//! Basic Call Center Server Example
//!
//! This example demonstrates a complete, working call center that:
//! 1. Accepts incoming customer calls
//! 2. Queues them if no agents are available
//! 3. Routes calls to agents when they become available
//! 4. Handles agent registration via SIP REGISTER

use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{info, error};

use rvoip_call_engine::{
    prelude::*,
    CallCenterServer, CallCenterServerBuilder,
    CallCenterConfig,
};

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .init();

    info!("🚀 Starting Basic Call Center Server");

    // Step 1: Configure the call center
    let mut config = CallCenterConfig::default();
    // Update the SIP bind address
    config.general.local_signaling_addr = "0.0.0.0:5060".parse()
        .map_err(|e| format!("Failed to parse address: {}", e))?;
    // Use IP address for test environment (no DNS resolution needed)
    config.general.domain = "127.0.0.1".to_string();
    config.agents.default_max_concurrent_calls = 1;

    // Step 2: Create the call center server using builder with in-memory database
    let mut server = CallCenterServerBuilder::new()
        .with_config(config)
        .with_database_path(":memory:".to_string())  // Use in-memory database
        .build()
        .await
        .map_err(|e| format!("Failed to create server: {}", e))?;

    info!("✅ Server created with in-memory database");

    // Step 3: Start the server
    server.start().await
        .map_err(|e| format!("Failed to start server: {}", e))?;

    // Step 4: Add test agents
    let agents = vec![
        ("alice", "Alice Smith", "support"),
        ("bob", "Bob Johnson", "support"),
    ];
    server.create_test_agents(agents).await?;

    // Step 5: Create default queues
    server.create_default_queues().await?;

    // Step 6: Run the server
    // The server.run() method will run indefinitely and handle everything
    server.run().await
        .map_err(|e| format!("Server error: {}", e))?;

    Ok(())
} 