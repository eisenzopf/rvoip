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
    api::{AdminApi, SupervisorApi},
    agent::{Agent, AgentId, AgentStatus},
};

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .init();

    info!("🚀 Starting Basic Call Center Server");

    // Step 1: Create database and configuration
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

    // Step 3: Create and start the call center engine
    let engine = CallCenterEngine::new(config.clone(), database).await
        .map_err(|e| format!("Failed to create engine: {}", e))?;
    info!("✅ Call center engine started on {}", config.general.local_signaling_addr);
    
    // Start event monitoring
    engine.clone().start_event_monitoring().await
        .map_err(|e| format!("Failed to start monitoring: {}", e))?;
    info!("✅ Started monitoring for REGISTER and other events");

    // Step 4: Create API instances
    let admin_api = AdminApi::new(engine.clone());
    let supervisor_api = SupervisorApi::new(engine.clone());

    // Step 5: Add test agents
    create_test_agents(&admin_api).await?;
    info!("✅ Test agents created");

    // Step 6: Create default queues
    admin_api.create_queue("support_queue").await
        .map_err(|e| format!("Failed to create support queue: {}", e))?;
    admin_api.create_queue("sales_queue").await
        .map_err(|e| format!("Failed to create sales queue: {}", e))?;
    info!("✅ Default queues created");

    // Step 7: Start monitoring for events
    let monitor_supervisor = supervisor_api.clone();
    tokio::spawn(async move {
        monitor_call_events(monitor_supervisor).await;
    });

    // Step 8: Display usage instructions
    println!("\n📞 CALL CENTER IS READY!");
    println!("=======================");
    println!("\n🔧 Configuration:");
    println!("  - SIP Address: {}", config.general.local_signaling_addr);
    println!("  - Domain: {}", config.general.domain);
    println!("\n👥 Test Agents (pre-configured in database):");
    println!("  - Alice: sip:alice@callcenter.example.com");
    println!("  - Bob: sip:bob@callcenter.example.com");
    println!("  - Charlie: sip:charlie@callcenter.example.com");
    println!("\n📋 How to Test:");
    println!("  1. Configure agent SIP phones to register as alice/bob/charlie");
    println!("  2. Point them to this server ({})", config.general.local_signaling_addr);
    println!("  3. Once registered, they'll show as 'available'");
    println!("  4. Make test calls to sip:support@callcenter.example.com");
    println!("  5. Calls will be routed to available agents");
    println!("\n🛑 Press Ctrl+C to stop the server\n");

    // Keep the server running
    loop {
        sleep(Duration::from_secs(60)).await;
        
        // Periodically display stats
        let stats = supervisor_api.get_stats().await;
        info!("📊 Stats - Active Calls: {}, Queued: {}, Agents Available: {}", 
              stats.active_calls, stats.queued_calls, stats.available_agents);
    }
}

/// Create test agents using the Admin API
async fn create_test_agents(admin_api: &AdminApi) -> std::result::Result<(), Box<dyn std::error::Error>> {
    let agents = vec![
        ("alice", "Alice Smith", "support"),
        ("bob", "Bob Johnson", "support"),
        ("charlie", "Charlie Brown", "sales"),
    ];

    for (username, name, department) in agents {
        let agent = Agent {
            id: AgentId::from(format!("agent_{}", username)),
            sip_uri: format!("sip:{}@callcenter.example.com", username),
            display_name: name.to_string(),
            skills: vec!["english".to_string(), department.to_string()],
            max_concurrent_calls: 1,
            status: AgentStatus::Offline,
            department: Some(department.to_string()),
            extension: None,
        };

        admin_api.add_agent(agent.clone()).await
            .map_err(|e| format!("Failed to add agent {}: {}", name, e))?;
        info!("Created agent: {} ({})", name, agent.sip_uri);
    }

    Ok(())
}

/// Monitor and log call events
async fn monitor_call_events(supervisor_api: SupervisorApi) {
    info!("👀 Starting event monitor");
    
    loop {
        sleep(Duration::from_secs(10)).await;
        
        // Get current queue stats
        match supervisor_api.get_all_queue_stats().await {
            Ok(queue_stats) => {
                for (queue_id, stats) in queue_stats {
                    if stats.total_calls > 0 {
                        info!("📊 Queue '{}' - Waiting: {}, Avg Wait: {}s", 
                              queue_id, stats.total_calls, stats.average_wait_time_seconds);
                    }
                }
            }
            Err(e) => error!("Failed to get queue stats: {}", e),
        }
        
        // Get agent status
        let agents = supervisor_api.list_agents().await;
        let available = agents.iter().filter(|a| matches!(a.status, AgentStatus::Available)).count();
        let busy = agents.iter().filter(|a| matches!(a.status, AgentStatus::Busy { .. })).count();
        
        if available > 0 || busy > 0 {
            info!("👥 Agents - Available: {}, Busy: {}", available, busy);
        }
    }
} 