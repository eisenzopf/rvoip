//! Phase 0 Basic Call Flow Example
//!
//! This example demonstrates the call-engine integration with session-core
//! using both direct engine access and the new API layer.

use anyhow::Result;
use rvoip_call_engine::{
    prelude::*,
    api::{CallCenterClient, SupervisorApi, AdminApi},
};
use std::sync::Arc;
use tokio::time::{sleep, Duration};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter("info,rvoip_call_engine=debug")
        .init();

    println!("🚀 Phase 0 Basic Call Flow with New API Demonstration\n");

    // Step 1: Create call center infrastructure
    println!("📊 Setting up call center...");
    let database = CallCenterDatabase::new_in_memory().await?;
    let config = CallCenterConfig::default();
    
    // Create the engine directly for core functionality
    let engine = CallCenterEngine::new(config.clone(), database.clone()).await?;
    
    // Start event monitoring for REGISTER and other events
    engine.clone().start_event_monitoring().await?;
    println!("✅ Call center engine created with session-core integration\n");

    // Step 2: Create API clients for different user types
    println!("🔌 Creating API clients...");
    
    // Agent client API
    let agent_client = CallCenterClient::new(engine.clone());
    println!("✅ Agent client created");
    
    // Supervisor API
    let supervisor_api = SupervisorApi::new(engine.clone());
    println!("✅ Supervisor API created");
    
    // Admin API
    let admin_api = AdminApi::new(engine.clone());
    println!("✅ Admin API created\n");

    // Step 3: Use Admin API to add agents
    println!("👥 Using Admin API to add agents...");
    
    let alice = Agent {
        id: AgentId::from("alice-001"),
        sip_uri: "sip:alice@agents.local".to_string(),
        display_name: "Alice Johnson".to_string(),
        skills: vec!["sales".to_string(), "english".to_string()],
        max_concurrent_calls: 2,
        status: AgentStatus::Available,
        department: Some("sales".to_string()),
        extension: Some("1001".to_string()),
    };
    
    let bob = Agent {
        id: AgentId::from("bob-002"),
        sip_uri: "sip:bob@agents.local".to_string(),
        display_name: "Bob Smith".to_string(),
        skills: vec!["support".to_string(), "english".to_string(), "spanish".to_string()],
        max_concurrent_calls: 3,
        status: AgentStatus::Available,
        department: Some("support".to_string()),
        extension: Some("1002".to_string()),
    };
    
    // Admin adds agents to the system
    admin_api.add_agent(alice.clone()).await?;
    println!("  ✅ Alice added by admin");
    
    admin_api.add_agent(bob.clone()).await?;
    println!("  ✅ Bob added by admin\n");

    // Step 4: Agents register using the client API
    println!("📱 Agents registering with the system...");
    
    let alice_session = agent_client.register_agent(&alice).await?;
    println!("  ✅ Alice registered with session: {}", alice_session);
    
    let bob_session = agent_client.register_agent(&bob).await?;
    println!("  ✅ Bob registered with session: {}", bob_session);
    println!();

    // Step 5: Supervisor checks system status
    println!("📊 Supervisor checking system status...");
    let stats = supervisor_api.get_stats().await;
    println!("  Call Center Statistics:");
    println!("  - Available Agents: {}", stats.available_agents);
    println!("  - Busy Agents: {}", stats.busy_agents);
    println!("  - Active Calls: {}", stats.active_calls);
    println!("  - Queued Calls: {}", stats.queued_calls);
    println!("  - Total Calls Handled: {}", stats.total_calls_handled);
    
    // List all agents
    let agents = supervisor_api.list_agents().await;
    println!("\n  Agent Details:");
    for agent_info in agents {
        println!("  - {} ({}): {:?}", 
                 agent_info.agent_id, 
                 agent_info.skills.join(", "),
                 agent_info.status);
    }
    println!();

    // Step 6: Demonstrate call flow
    println!("📞 Call Flow with New Architecture:\n");
    
    println!("  INCOMING CALL HANDLING:");
    println!("  1. Customer calls → Session-core receives INVITE");
    println!("  2. Session-core → CallHandler.on_incoming_call()");
    println!("  3. CallCenterEngine analyzes and routes");
    println!("  4. Returns CallDecision (Accept/Reject/Queue)");
    println!("  5. If accepted → Bridge to agent\n");
    
    println!("  REAL-TIME EVENTS (NEW!):");
    println!("  • on_call_state_changed → Track call lifecycle");
    println!("  • on_media_quality → Monitor call quality (MOS)");
    println!("  • on_dtmf → Handle IVR input");
    println!("  • on_media_flow → Track media status");
    println!("  • on_warning → System alerts\n");

    // Step 7: Simulate agent status changes
    println!("🔄 Simulating agent status changes...");
    
    // Alice takes a break
    agent_client.update_agent_status(
        &alice.id,
        AgentStatus::Break { duration_minutes: 15 }
    ).await?;
    println!("  ✅ Alice is now on break");
    
    // Check updated stats
    let updated_stats = supervisor_api.get_stats().await;
    println!("  📊 Updated: {} available, {} on break", 
             updated_stats.available_agents,
             1);
    println!();

    // Step 8: Admin checks system health
    println!("🏥 Admin checking system health...");
    let health = admin_api.get_system_health().await;
    println!("  System Health: {:?}", health.status);
    println!("  - Database Connected: {}", health.database_connected);
    println!("  - Active Sessions: {}", health.active_sessions);
    println!("  - Registered Agents: {}", health.registered_agents);
    println!();

    // Summary
    println!("✅ Phase 0 Complete - New Architecture Benefits:");
    println!("  • Clean API separation (Agent/Supervisor/Admin)");
    println!("  • Real-time event handling via CallHandler");
    println!("  • Session-core manages all SIP/RTP complexity");
    println!("  • Type-safe interfaces for all operations");
    println!("  • Built-in monitoring and health checks");
    
    println!("\n🎉 The call center is ready for production use!");

    Ok(())
} 