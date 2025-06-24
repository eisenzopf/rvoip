//! Agent Registration Demo using CallCenterClient API
//!
//! This example demonstrates how to use the new CallCenterClient API
//! for agent registration and management in the call center.

use anyhow::Result;
use rvoip_call_engine::{
    prelude::*,
    api::CallCenterClient,
    agent::{Agent, AgentId, AgentStatus},
};
use std::sync::Arc;
use tokio::time::{sleep, Duration};
use tracing::info;

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter("info")
        .init();

    println!("🎯 Agent Registration Demo - Using CallCenterClient API\n");

    // Step 1: Create database and configuration
    println!("📦 Setting up call center infrastructure...");
    let database = CallCenterDatabase::new_in_memory().await?;
    let config = CallCenterConfig::default();
    
    // Step 2: Build the CallCenterClient
    let engine = CallCenterEngine::new(config, database).await?;
    let client = CallCenterClient::new(engine.clone());
    
    println!("✅ CallCenterClient created successfully!\n");

    // Step 3: Register Agent Alice
    println!("👤 Registering Agent Alice...");
    let alice = Agent {
        id: AgentId::from("alice-001"),
        sip_uri: "sip:alice@callcenter.example.com".to_string(),
        display_name: "Alice Smith".to_string(),
        skills: vec!["english".to_string(), "sales".to_string(), "support".to_string()],
        max_concurrent_calls: 3,
        status: AgentStatus::Available,
        department: Some("sales".to_string()),
        extension: Some("1001".to_string()),
    };
    
    let alice_session = client.register_agent(&alice).await?;
    println!("✅ Alice registered with session: {}", alice_session);
    println!("   - Skills: {:?}", alice.skills);
    println!("   - Max concurrent calls: {}", alice.max_concurrent_calls);

    // Step 4: Register Agent Bob
    println!("\n👤 Registering Agent Bob...");
    let bob = Agent {
        id: AgentId::from("bob-002"),
        sip_uri: "sip:bob@callcenter.example.com".to_string(),
        display_name: "Bob Johnson".to_string(),
        skills: vec!["english".to_string(), "spanish".to_string(), "support".to_string()],
        max_concurrent_calls: 2,
        status: AgentStatus::Available,
        department: Some("support".to_string()),
        extension: Some("1002".to_string()),
    };
    
    let bob_session = client.register_agent(&bob).await?;
    println!("✅ Bob registered with session: {}", bob_session);

    // Step 5: Check agent information
    println!("\n📊 Checking agent information...");
    if let Some(alice_info) = client.get_agent_info(&alice.id).await {
        println!("Alice's current info:");
        println!("  - Status: {:?}", alice_info.status);
        println!("  - Active calls: {}", alice_info.current_calls);
        println!("  - Performance score: {:.2}", alice_info.performance_score);
    }

    // Step 6: Update agent status
    println!("\n🔄 Updating agent statuses...");
    
    // Alice goes on break
    client.update_agent_status(
        &alice.id, 
        AgentStatus::Break { duration_minutes: 15 }
    ).await?;
    println!("✅ Alice is now on a 15-minute break");
    
    // Bob becomes busy
    client.update_agent_status(
        &bob.id,
        AgentStatus::Busy { active_calls: 1 }
    ).await?;
    println!("✅ Bob is now busy with 1 active call");

    // Step 7: Check queue statistics
    println!("\n📈 Checking queue statistics...");
    let queue_stats = client.get_queue_stats().await?;
    for (queue_id, stats) in queue_stats {
        println!("Queue '{}': {} calls waiting", queue_id, stats.total_calls);
    }

    // Step 8: Demonstrate session-core integration
    println!("\n🔌 Session-Core Integration:");
    let session_manager = client.session_manager();
    println!("✅ Direct access to SessionCoordinator available");
    println!("   - Can handle incoming calls via CallHandler");
    println!("   - Can create outgoing calls for agents");
    println!("   - Manages all SIP transport internally");

    // Step 9: Agent logout simulation
    println!("\n📴 Simulating agent logout...");
    client.update_agent_status(&alice.id, AgentStatus::Offline).await?;
    println!("✅ Alice is now offline");

    // Summary
    println!("\n📋 Summary - CallCenterClient API Benefits:");
    println!("✅ Simple, type-safe agent management");
    println!("✅ Integrated with session-core for SIP handling");
    println!("✅ Real-time status updates");
    println!("✅ Performance tracking built-in");
    println!("✅ Queue statistics and monitoring");
    println!("✅ Clean separation of concerns");
    
    println!("\n🎉 Demo completed successfully!");

    Ok(())
} 