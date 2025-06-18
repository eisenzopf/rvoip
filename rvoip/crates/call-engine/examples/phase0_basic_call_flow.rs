//! Phase 0 Basic Call Flow Example
//!
//! This example demonstrates how the call-engine now properly integrates with
//! session-core using the CallHandler trait to receive and route incoming calls.

use anyhow::Result;
use rvoip_call_engine::prelude::*;
use std::sync::Arc;
use tokio::time::{sleep, Duration};

#[tokio::main]
async fn main() -> Result<()> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_env_filter("info,rvoip_call_engine=debug")
        .init();

    println!("🚀 Phase 0 Basic Call Flow Demonstration\n");

    // Step 1: Initialize database
    println!("📊 Initializing database...");
    let database = CallCenterDatabase::new_in_memory().await?;
    println!("✅ Database initialized\n");

    // Step 2: Create call center configuration
    println!("⚙️ Creating call center configuration...");
    let config = CallCenterConfig::default();
    println!("✅ Configuration ready\n");

    // Step 3: Create CallCenterEngine with proper CallHandler integration
    println!("🎯 Creating CallCenterEngine with CallHandler integration...");
    let call_center = CallCenterEngine::new(config, database).await?;
    println!("✅ CallCenterEngine created with session-core integration!\n");

    // Step 4: Register some agents
    println!("👥 Registering agents...");
    
    let agents = vec![
        Agent {
            id: "alice-001".to_string(),
            sip_uri: "sip:alice@agents.local".parse()?,
            display_name: "Alice Johnson".to_string(),
            skills: vec!["sales".to_string(), "english".to_string()],
            max_concurrent_calls: 2,
            status: AgentStatus::Available,
            department: Some("sales".to_string()),
            extension: Some("1001".to_string()),
        },
        Agent {
            id: "bob-002".to_string(),
            sip_uri: "sip:bob@agents.local".parse()?,
            display_name: "Bob Smith".to_string(),
            skills: vec!["support".to_string(), "english".to_string()],
            max_concurrent_calls: 3,
            status: AgentStatus::Available,
            department: Some("support".to_string()),
            extension: Some("1002".to_string()),
        },
    ];
    
    for agent in &agents {
        let session_id = call_center.register_agent(agent).await?;
        println!("  ✅ Registered {} ({})", agent.display_name, session_id);
    }
    println!();

    // Step 5: Display current statistics
    println!("📊 Call Center Statistics:");
    let stats = call_center.get_stats().await;
    println!("  - Available Agents: {}", stats.available_agents);
    println!("  - Busy Agents: {}", stats.busy_agents);
    println!("  - Active Calls: {}", stats.active_calls);
    println!("  - Queued Calls: {}", stats.queued_calls);
    println!();

    // Step 6: Explain the call flow
    println!("📞 How Incoming Calls Work Now:\n");
    println!("  1. Customer dials into the call center");
    println!("  2. Session-core receives the SIP INVITE");
    println!("  3. Session-core calls our CallHandler.on_incoming_call()");
    println!("  4. CallCenterEngine processes the call with routing logic:");
    println!("     - Analyzes customer info (phone number patterns)");
    println!("     - Determines required skills");
    println!("     - Finds available agent or queues the call");
    println!("  5. Returns CallDecision to session-core");
    println!("  6. If accepted, call is bridged to selected agent");
    println!();

    // Step 7: Simulate what happens when calls arrive
    println!("🔄 When calls arrive, the CallHandler will:");
    println!("  - Route sales calls to Alice");
    println!("  - Route support calls to Bob");
    println!("  - Queue calls if agents are busy");
    println!("  - Reject calls if queues are full");
    println!();

    // Note: In a real scenario, you would need to keep the application running
    // to receive actual SIP calls. For now, we'll just demonstrate the setup.
    
    println!("✅ Phase 0 integration complete!");
    println!("   The call center is now ready to receive calls via session-core.");
    println!("   Deploy with a real SIP endpoint to test actual call flows.");

    Ok(())
} 