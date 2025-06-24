//! Phase 0: Basic Call Flow Demo
//!
//! This example demonstrates the high-level API usage of the call-engine crate.
//! It shows how to use CallCenterClient, SupervisorApi, and AdminApi.

use rvoip_call_engine::prelude::*;
use rvoip_call_engine::{
    CallCenterServer, CallCenterServerBuilder,
    CallCenterConfig,
    agent::{Agent, AgentId, AgentStatus},
};
use tracing::info;
use std::time::Duration;
use tokio::time::sleep;

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .init();

    info!("🚀 Starting Phase 0 Basic Call Flow Demo");

    // Create database and configuration
    let database = CallCenterDatabase::new_in_memory().await
        .map_err(|e| format!("Failed to create database: {}", e))?;
    let mut config = CallCenterConfig::default();
    config.general.domain = "example.com".to_string();

    // Create server using builder pattern
    let mut server = CallCenterServerBuilder::new()
        .with_config(config)
        .with_database(database)
        .build()
        .await
        .map_err(|e| format!("Failed to build server: {}", e))?;

    // Start the server
    server.start().await
        .map_err(|e| format!("Failed to start server: {}", e))?;

    // Example 1: Admin API - Add agents
    info!("\n📋 Example 1: Adding agents using AdminApi");
    let admin_api = server.admin_api();
    
    let alice = Agent {
        id: AgentId::from("alice"),
        sip_uri: "sip:alice@example.com".to_string(),
        display_name: "Alice Smith".to_string(),
        skills: vec!["english".to_string(), "support".to_string()],
        max_concurrent_calls: 2,
        status: AgentStatus::Offline,
        department: Some("support".to_string()),
        extension: Some("1001".to_string()),
    };
    
    admin_api.add_agent(alice.clone()).await
        .map_err(|e| format!("Failed to add agent: {}", e))?;
    info!("✅ Agent Alice added");

    // Example 2: CallCenterClient - Agent registration
    info!("\n📞 Example 2: Agent registration using CallCenterClient");
    let alice_client = server.create_client("alice".to_string());
    
    // First register the agent
    alice_client.register_agent(&alice).await
        .map_err(|e| format!("Failed to register agent: {}", e))?;
    info!("✅ Alice registered");
    
    // Update status to available
    alice_client.update_agent_status(&alice.id, AgentStatus::Available).await
        .map_err(|e| format!("Failed to update status: {}", e))?;
    info!("✅ Alice is now available");

    // Example 3: SupervisorApi - Monitor system
    info!("\n👀 Example 3: System monitoring using SupervisorApi");
    let supervisor_api = server.supervisor_api();
    
    let stats = supervisor_api.get_stats().await;
    info!("📊 System stats: {:?}", stats);
    
    let agents = supervisor_api.list_agents().await;
    info!("👥 Active agents: {}", agents.len());
    for agent in &agents {
        info!("  - {} ({}): {:?}", agent.agent_id, agent.agent_id, agent.status);
    }

    // Example 4: Queue management
    info!("\n📋 Example 4: Queue management");
    admin_api.create_queue("support_queue").await
        .map_err(|e| format!("Failed to create queue: {}", e))?;
    info!("✅ Support queue created");
    
    let all_queue_stats = supervisor_api.get_all_queue_stats().await
        .map_err(|e| format!("Failed to get queue stats: {}", e))?;
    for (queue_id, stats) in all_queue_stats {
        if queue_id == "support_queue" {
            info!("📊 Support queue stats: {:?}", stats);
        }
    }

    // Keep running for a bit to show monitoring
    info!("\n🔄 Running for 10 seconds to demonstrate monitoring...");
    sleep(Duration::from_secs(10)).await;
    
    info!("\n✅ Demo completed successfully!");
    Ok(())
} 