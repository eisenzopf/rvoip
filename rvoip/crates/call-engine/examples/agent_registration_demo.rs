//! Agent Registration Demo
//!
//! This example demonstrates how agents can register with the call center
//! using the CallCenterClient API.

use rvoip_call_engine::{
    prelude::*,
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

    info!("🚀 Starting Agent Registration Demo");

    // Create configuration and database
    let mut config = CallCenterConfig::default();
    config.general.domain = "callcenter.example.com".to_string();
    config.general.local_signaling_addr = "0.0.0.0:5060".parse()
        .map_err(|e| format!("Failed to parse address: {}", e))?;
    
    let database = CallCenterDatabase::new_in_memory().await
        .map_err(|e| format!("Failed to create database: {}", e))?;

    // Create and start server
    let mut server = CallCenterServerBuilder::new()
        .with_config(config)
        .with_database(database)
        .build()
        .await
        .map_err(|e| format!("Failed to build server: {}", e))?;
    
    server.start().await
        .map_err(|e| format!("Failed to start server: {}", e))?;
    info!("✅ Call center server started");

    // Create test agents using admin API
    let admin_api = server.admin_api();
    
    // Add Alice
    let alice = Agent {
        id: AgentId::from("agent_alice"),
        sip_uri: "sip:alice@callcenter.example.com".to_string(),
        display_name: "Alice Smith".to_string(),
        skills: vec!["english".to_string(), "support".to_string()],
        max_concurrent_calls: 2,
        status: AgentStatus::Offline,
        department: Some("support".to_string()),
        extension: Some("1001".to_string()),
    };
    admin_api.add_agent(alice.clone()).await
        .map_err(|e| format!("Failed to add Alice: {}", e))?;
    info!("✅ Agent Alice added to system");

    // Add Bob
    let bob = Agent {
        id: AgentId::from("agent_bob"),
        sip_uri: "sip:bob@callcenter.example.com".to_string(),
        display_name: "Bob Johnson".to_string(),
        skills: vec!["english".to_string(), "sales".to_string()],
        max_concurrent_calls: 1,
        status: AgentStatus::Offline,
        department: Some("sales".to_string()),
        extension: Some("1002".to_string()),
    };
    admin_api.add_agent(bob.clone()).await
        .map_err(|e| format!("Failed to add Bob: {}", e))?;
    info!("✅ Agent Bob added to system");

    // Create queues
    admin_api.create_queue("support_queue").await
        .map_err(|e| format!("Failed to create support queue: {}", e))?;
    admin_api.create_queue("sales_queue").await
        .map_err(|e| format!("Failed to create sales queue: {}", e))?;
    info!("✅ Queues created");

    // Demonstrate agent registration using client API
    info!("\n📱 Starting agent registration process...");

    // Alice registers and becomes available
    let alice_client = server.create_client("agent_alice".to_string());
    
    alice_client.register_agent(&alice).await
        .map_err(|e| format!("Failed to register Alice: {}", e))?;
    info!("✅ Alice registered with the system");
    
    alice_client.update_agent_status(&alice.id, AgentStatus::Available).await
        .map_err(|e| format!("Failed to update Alice status: {}", e))?;
    info!("✅ Alice is now available for calls");

    // Bob registers but stays offline initially
    let bob_client = server.create_client("agent_bob".to_string());
    
    bob_client.register_agent(&bob).await
        .map_err(|e| format!("Failed to register Bob: {}", e))?;
    info!("✅ Bob registered with the system");

    // Check system status
    let supervisor_api = server.supervisor_api();
    let stats = supervisor_api.get_stats().await;
    info!("\n📊 System Status:");
    info!("  - Available agents: {}", stats.available_agents);
    info!("  - Offline agents: {}", stats.busy_agents + (2 - stats.available_agents - stats.busy_agents));
    info!("  - Active calls: {}", stats.active_calls);

    // Simulate Bob becoming available
    sleep(Duration::from_secs(2)).await;
    bob_client.update_agent_status(&bob.id, AgentStatus::Available).await
        .map_err(|e| format!("Failed to update Bob status: {}", e))?;
    info!("\n✅ Bob is now available for calls");

    // Check updated status
    let stats = supervisor_api.get_stats().await;
    info!("\n📊 Updated System Status:");
    info!("  - Available agents: {}", stats.available_agents);

    // List all agents
    let agents = supervisor_api.list_agents().await;
    info!("\n👥 Registered Agents:");
    for agent in agents {
        let status = match agent.status {
            AgentStatus::Available => "Available ✅",
            AgentStatus::Offline => "Offline ⭕",
            AgentStatus::Busy { .. } => "Busy 🔴",
            AgentStatus::Break { .. } => "On Break ☕",
            AgentStatus::Away { .. } => "Away 🚪",
        };
        info!("  - {} ({}): {}", agent.agent_id, agent.agent_id, status);
    }

    // Demonstrate agent going on break
    sleep(Duration::from_secs(2)).await;
    alice_client.update_agent_status(&alice.id, AgentStatus::Break { duration_minutes: 15 }).await
        .map_err(|e| format!("Failed to update Alice to break: {}", e))?;
    info!("\n☕ Alice is taking a 15-minute break");

    // Check queue stats
    let all_queue_stats = supervisor_api.get_all_queue_stats().await
        .map_err(|e| format!("Failed to get queue stats: {}", e))?;
    info!("\n📊 Queue Stats:");
    for (queue_id, stats) in all_queue_stats {
        if queue_id == "support_queue" {
            info!("  Support Queue - Calls waiting: {}, Avg wait time: {}s",
                  stats.total_calls, stats.average_wait_time_seconds);
        }
    }

    info!("\n✅ Agent registration demo completed!");
    info!("💡 In a real deployment, agents would register via SIP REGISTER messages");
    
    Ok(())
} 