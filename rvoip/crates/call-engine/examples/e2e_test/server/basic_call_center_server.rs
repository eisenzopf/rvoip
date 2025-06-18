//! Basic Call Center Server Example
//!
//! This example demonstrates a complete, working call center that:
//! 1. Accepts incoming customer calls
//! 2. Queues them if no agents are available
//! 3. Routes calls to agents when they become available
//! 4. Handles agent registration via SIP REGISTER

use std::error::Error;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use tokio::time::sleep;
use tracing::{info, error};

use rvoip_call_engine::{
    CallCenterEngine,
    config::{CallCenterConfig, QueueConfig, AgentConfig},
    agent::AgentStatus,
    database::DatabaseManager,
};

#[tokio::main]
async fn main() -> Result<(), Box<dyn Error>> {
    // Initialize logging
    tracing_subscriber::fmt()
        .with_max_level(tracing::Level::INFO)
        .with_target(false)
        .init();

    info!("🚀 Starting Basic Call Center Server");

    // Step 1: Create database and schema
    let db_manager = DatabaseManager::new("call_center.db").await?;
    db_manager.create_schema().await?;
    info!("✅ Database initialized");

    // Step 2: Add some test agents to the database
    create_test_agents(&db_manager).await?;
    info!("✅ Test agents created");

    // Step 3: Configure the call center
    let config = CallCenterConfig {
        sip_bind_address: "0.0.0.0:5060".parse()?,
        domain: "callcenter.example.com".to_string(),
        max_calls_per_agent: 1,
        call_timeout: Duration::from_secs(30),
        ..Default::default()
    };

    // Step 4: Create and start the call center engine
    let engine = Arc::new(CallCenterEngine::new(config.clone()).await?);
    info!("✅ Call center engine started on {}", config.sip_bind_address);

    // Step 5: Create a default queue for incoming calls
    create_default_queue(&engine).await?;
    info!("✅ Default queue created");

    // Step 6: Start monitoring for events
    let monitor_engine = engine.clone();
    tokio::spawn(async move {
        monitor_call_events(monitor_engine).await;
    });

    // Step 7: Display usage instructions
    println!("\n📞 CALL CENTER IS READY!");
    println!("=======================");
    println!("\n🔧 Configuration:");
    println!("  - SIP Address: {}", config.sip_bind_address);
    println!("  - Domain: {}", config.domain);
    println!("\n👥 Test Agents (pre-configured in database):");
    println!("  - Alice: sip:alice@callcenter.example.com");
    println!("  - Bob: sip:bob@callcenter.example.com");
    println!("  - Charlie: sip:charlie@callcenter.example.com");
    println!("\n📋 How to Test:");
    println!("  1. Configure agent SIP phones to register as alice/bob/charlie");
    println!("  2. Point them to this server ({})", config.sip_bind_address);
    println!("  3. Once registered, they'll show as 'available'");
    println!("  4. Make test calls to sip:support@callcenter.example.com");
    println!("  5. Calls will be routed to available agents");
    println!("\n🛑 Press Ctrl+C to stop the server\n");

    // Keep the server running
    loop {
        sleep(Duration::from_secs(60)).await;
        
        // Periodically display stats
        let stats = engine.get_stats().await;
        info!("📊 Stats - Active Calls: {}, Queued: {}, Agents Online: {}", 
              stats.active_calls, stats.queued_calls, stats.agents_online);
    }
}

/// Create test agents in the database
async fn create_test_agents(db: &DatabaseManager) -> Result<(), Box<dyn Error>> {
    let agents = vec![
        ("alice", "Alice Smith", "support"),
        ("bob", "Bob Johnson", "support"),
        ("charlie", "Charlie Brown", "sales"),
    ];

    for (username, name, department) in agents {
        let agent_config = AgentConfig {
            id: format!("agent_{}", username),
            sip_uri: format!("sip:{}@callcenter.example.com", username),
            display_name: name.to_string(),
            max_concurrent_calls: 1,
            skills: vec!["english".to_string(), department.to_string()],
            department: Some(department.to_string()),
            extension: None,
        };

        db.create_agent(&agent_config).await?;
        info!("Created agent: {} ({})", name, agent_config.sip_uri);
    }

    Ok(())
}

/// Create a default queue for incoming calls
async fn create_default_queue(engine: &CallCenterEngine) -> Result<(), Box<dyn Error>> {
    let queue_config = QueueConfig {
        id: "support_queue".to_string(),
        name: "Support Queue".to_string(),
        max_wait_time: Duration::from_secs(300), // 5 minutes
        max_size: 50,
        overflow_action: "voicemail".to_string(),
        routing_strategy: "round_robin".to_string(),
        priority: 1,
    };

    // Create queue using internal API
    let agent_manager = engine.agent_manager();
    let mut agent_mgr = agent_manager.lock().await;
    agent_mgr.queues.create_queue(
        queue_config.id.clone(),
        queue_config.name.clone(),
        queue_config.max_size,
    )?;

    Ok(())
}

/// Monitor and log call events
async fn monitor_call_events(engine: Arc<CallCenterEngine>) {
    info!("👀 Starting event monitor");
    
    loop {
        sleep(Duration::from_secs(10)).await;
        
        // Get current queue stats
        match engine.get_queue_stats("support_queue").await {
            Ok(stats) => {
                if stats.calls_in_queue > 0 || stats.agents_available > 0 {
                    info!("📊 Queue Stats - Waiting: {}, Available Agents: {}", 
                          stats.calls_in_queue, stats.agents_available);
                }
            }
            Err(e) => error!("Failed to get queue stats: {}", e),
        }
    }
} 