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

    // Step 1: Create database
    let database = CallCenterDatabase::new_in_memory().await
        .map_err(|e| format!("Failed to create database: {}", e))?;
    info!("✅ Database initialized");

    // Step 2: Configure the call center
    let mut config = CallCenterConfig::default();
    // Update the SIP bind address
    config.general.local_signaling_addr = "0.0.0.0:5060".parse()
        .map_err(|e| format!("Failed to parse address: {}", e))?;
    config.general.domain = "callcenter.example.com".to_string();
    config.agents.default_max_concurrent_calls = 1;

    // Step 3: Create the call center server using builder
    let mut server = CallCenterServerBuilder::new()
        .with_config(config)
        .with_database(database)
        .build()
        .await
        .map_err(|e| format!("Failed to create server: {}", e))?;

    // Step 4: Start the server
    server.start().await
        .map_err(|e| format!("Failed to start server: {}", e))?;

    // Step 5: Add test agents
    let agents = vec![
        ("alice", "Alice Smith", "support"),
        ("bob", "Bob Johnson", "support"),
        ("charlie", "Charlie Brown", "sales"),
    ];
    server.create_test_agents(agents).await?;

    // Step 6: Create default queues
    server.create_default_queues().await?;

    // Step 7: Run the server
    // The server.run() method will run indefinitely and handle everything
    server.run().await
        .map_err(|e| format!("Server error: {}", e))?;

    Ok(())
} 