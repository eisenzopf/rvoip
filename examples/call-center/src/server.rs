#![allow(warnings)]

//! Call Center Server
//!
//! This is a minimal call center server that:
//! 1. Accepts incoming customer calls to sip:support@127.0.0.1
//! 2. Routes calls to available agents
//! 3. Handles agent registration via SIP REGISTER

use tracing::{info, error};

use rvoip_call_engine::{
    prelude::*,
    CallCenterServerBuilder,
    CallCenterConfig,
};

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    // Initialize logging with file output
    let file_appender = tracing_appender::rolling::never("logs", "server.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
    
    tracing_subscriber::fmt()
        .with_writer(non_blocking)
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("call_center_demo=info".parse()?)
                .add_directive("rvoip_call_engine=info".parse()?)
        )
        .init();

    info!("🏢 Starting Call Center Server");
    info!("📞 Support Line: sip:support@127.0.0.1");
    info!("🔗 SIP Address: 0.0.0.0:5060");

    // Step 1: Configure the call center with minimal settings
    let mut config = CallCenterConfig::default();
    config.general.local_signaling_addr = "0.0.0.0:5060".parse()?;
    config.general.domain = "127.0.0.1".to_string();
    config.agents.default_max_concurrent_calls = 1;
    
    // Set queue parameters for demo
    config.queues.default_max_wait_time = 60; // 60 seconds max wait
    config.queues.max_queue_size = 10; // Small queue for demo

    info!("⚙️  Server configuration:");
    info!("   Domain: {}", config.general.domain);
    info!("   Max calls per agent: {}", config.agents.default_max_concurrent_calls);
    info!("   Queue max wait time: {}s", config.queues.default_max_wait_time);

    // Step 2: Create the call center server with in-memory database
    let mut server = CallCenterServerBuilder::new()
        .with_config(config)
        .with_database_path(":memory:".to_string())
        .build()
        .await
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;

    info!("✅ Call center server created with in-memory database");

    // Step 3: Start the server
    server.start().await
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;
    info!("🚀 Call center server started successfully");

    // Step 4: Create default queues and wait for agents to register
    server.create_default_queues().await
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;
    info!("📋 Default call queues created");

    info!("👥 Waiting for agents to register...");
    info!("📞 Ready to accept customer calls on sip:support@127.0.0.1");

    // Step 5: Run the server (this will run indefinitely)
    // The server handles all SIP signaling, agent registration, and call routing
    match server.run().await {
        Ok(_) => info!("🏁 Server completed successfully"),
        Err(e) => {
            error!("❌ Server error: {}", e);
            return Err(Box::new(e) as Box<dyn std::error::Error>);
        }
    }

    Ok(())
} 