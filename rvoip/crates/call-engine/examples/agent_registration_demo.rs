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

    // Configure the call center
    let mut config = CallCenterConfig::default();
    config.general.local_signaling_addr = "127.0.0.1:5060".parse()?;

    // Create server with in-memory database
    let mut server = CallCenterServerBuilder::new()
        .with_config(config)
        .with_database_path(":memory:".to_string())
        .build()
        .await?;

    info!("✅ Call center server created");

    // Start the server
    server.start().await?;
    info!("🎯 Server listening on 127.0.0.1:5060");

    // Create test agents
    let alice = Agent {
        id: "agent_alice".to_string(),
        sip_uri: "sip:alice@127.0.0.1:5071".to_string(),
        display_name: "Alice Johnson".to_string(),
        skills: vec!["english".to_string(), "support".to_string()],
        max_concurrent_calls: 2,
        status: AgentStatus::Offline,
        department: Some("support".to_string()),
        extension: Some("101".to_string()),
    };

    let bob = Agent {
        id: "agent_bob".to_string(),
        sip_uri: "sip:bob@127.0.0.1:5072".to_string(),
        display_name: "Bob Smith".to_string(),
        skills: vec!["english".to_string(), "sales".to_string()],
        max_concurrent_calls: 3,
        status: AgentStatus::Offline,
        department: Some("sales".to_string()),
        extension: Some("102".to_string()),
    };

    // Create test agents using admin API
    let admin_api = server.admin_api();
    
    // Add Alice
    admin_api.add_agent(alice.clone()).await
        .map_err(|e| format!("Failed to add Alice: {}", e))?;
    info!("✅ Agent Alice added to system");

    // Add Bob
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
    
    // Update agent status
    info!("\n📱 Updating agent status");
    alice_client.update_agent_status(&AgentId(alice.id.clone()), AgentStatus::Available).await
        .expect("Failed to update Alice status");
    info!("✅ Alice is now available");
    
    // Check agent info
    if let Some(alice_info) = alice_client.get_agent_info(&AgentId(alice.id.clone())).await {
        info!("Alice info: {:?}", alice_info);
    }

    // Demonstrate different agent statuses
    info!("\n🔄 Demonstrating agent status changes");
    
    // Bob goes available
    let bob_client = server.create_client("agent_bob".to_string());
    
    bob_client.register_agent(&bob).await
        .map_err(|e| format!("Failed to register Bob: {}", e))?;
    info!("✅ Bob registered with the system");
    
    // Bob goes available
    bob_client.update_agent_status(&AgentId(bob.id.clone()), AgentStatus::Available).await
        .expect("Failed to update Bob status");
    info!("✅ Bob is now available");
    
    sleep(Duration::from_secs(2)).await;
    
    // Alice goes busy (simulating a call)
    alice_client.update_agent_status(&AgentId(alice.id.clone()), AgentStatus::Busy(vec![])).await
        .expect("Failed to update Alice status");
    info!("☎️ Alice is now busy");
    
    sleep(Duration::from_secs(2)).await;
    
    // Bob goes offline
    bob_client.update_agent_status(&AgentId(bob.id.clone()), AgentStatus::Offline).await
        .expect("Failed to update Bob status");
    info!("🚪 Bob is now offline");

    // Check system status
    let supervisor_api = server.supervisor_api();
    let stats = supervisor_api.get_stats().await;
    info!("\n📊 System Status:");
    info!("  - Available agents: {}", stats.available_agents);
    info!("  - Offline agents: {}", stats.busy_agents + (2 - stats.available_agents - stats.busy_agents));
    info!("  - Active calls: {}", stats.active_calls);

    // List agents with different statuses
    info!("\n📊 Current agent statuses:");
    let agents = admin_api.list_agents().await.expect("Failed to list agents");
    for agent in agents {
        let status = match agent.status {
            AgentStatus::Available => "Available ✅",
            AgentStatus::Busy(_) => "Busy 📞",
            AgentStatus::Offline => "Offline 🚪",
        };
        info!("  {} ({}): {}", agent.display_name, agent.id, status);
    }

    info!("\n✅ Agent registration demo completed!");
    info!("💡 In a real deployment, agents would register via SIP REGISTER messages");
    
    Ok(())
} 