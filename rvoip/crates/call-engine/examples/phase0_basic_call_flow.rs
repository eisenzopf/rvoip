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

    info!("🚀 Starting Basic Call Flow Demo");

    // Configure the call center
    let mut config = CallCenterConfig::default();
    config.general.local_signaling_addr = "127.0.0.1:5060".parse()?;

    // Create the call center server
    info!("Creating call center server...");
    let mut server = CallCenterServerBuilder::new()
        .with_config(config)
        .with_database_path(":memory:".to_string())  // Use in-memory database
        .build()
        .await?;

    info!("✅ Server created");

    // Step 1: Start the server
    server.start().await?;
    info!("🎯 Server listening on 127.0.0.1:5060");

    // Step 2: Create test agents
    let alice = Agent {
        id: "alice".to_string(),
        sip_uri: "sip:alice@127.0.0.1:5071".to_string(),
        display_name: "Alice Smith".to_string(),
        skills: vec!["english".to_string(), "sales".to_string()],
        max_concurrent_calls: 1,
        status: AgentStatus::Available,
        department: Some("sales".to_string()),
        extension: Some("101".to_string()),
    };

    // Example 1: Admin API - Add agents
    info!("\n📋 Example 1: Adding agents using AdminApi");
    let admin_api = server.admin_api();
    
    admin_api.add_agent(alice.clone()).await
        .map_err(|e| format!("Failed to add agent: {}", e))?;
    info!("✅ Agent Alice added");

    // Example 2: CallCenterClient API - Agent operations
    info!("\n📞 Example 2: Using CallCenterClient for agent operations");
    let alice_client = server.create_client("alice".to_string());
    
    alice_client.register_agent(&alice).await
        .expect("Failed to register Alice");
    info!("✅ Alice registered successfully");
    
    alice_client.update_agent_status(&AgentId(alice.id.clone()), AgentStatus::Available).await
        .expect("Failed to update Alice status");
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