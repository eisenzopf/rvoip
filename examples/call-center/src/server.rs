#![allow(warnings)]

//! Call Center Server
//!
//! This is a call center server that:
//! 1. Accepts incoming customer calls to sip:support@domain
//! 2. Routes calls to available agents
//! 3. Handles agent registration via SIP REGISTER
//! 4. Supports running on separate computers with configurable addresses

use tracing::{info, error};
use clap::Parser;
use std::net::SocketAddr;

use rvoip::call_engine::{prelude::*, CallCenterServerBuilder, CallCenterConfig};

#[derive(Parser, Debug)]
#[command(author, version, about = "Call Center Server", long_about = None)]
struct Args {
    /// Server bind address (IP:PORT)
    #[arg(short, long, default_value = "0.0.0.0:5060")]
    bind_addr: String,
    
    /// Public domain/IP for SIP communication
    #[arg(short, long, default_value = "127.0.0.1")]
    domain: String,
    
    /// Database path (use ":memory:" for in-memory database)
    #[arg(long, default_value = ":memory:")]
    database_path: String,
    
    /// Maximum concurrent calls per agent
    #[arg(long, default_value = "1")]
    max_calls_per_agent: u32,
    
    /// Maximum wait time in queue (seconds)
    #[arg(long, default_value = "60")]
    max_wait_time: u64,
    
    /// Maximum queue size
    #[arg(long, default_value = "10")]
    max_queue_size: usize,
    
    /// Enable verbose logging
    #[arg(short, long)]
    verbose: bool,
}

#[tokio::main]
async fn main() -> std::result::Result<(), Box<dyn std::error::Error>> {
    let args = Args::parse();
    
    // Create logs directory
    std::fs::create_dir_all("logs")?;
    
    // Initialize logging with file output
    let file_appender = tracing_appender::rolling::never("logs", "server.log");
    let (non_blocking, _guard) = tracing_appender::non_blocking(file_appender);
    
    let log_level = if args.verbose { "debug" } else { "info" };
    tracing_subscriber::fmt()
        .with_writer(non_blocking)
        .with_env_filter(
            tracing_subscriber::EnvFilter::from_default_env()
                .add_directive("call_center_demo=info".parse()?)
                .add_directive(format!("rvoip={}", log_level).parse()?)
        )
        .init();

    info!("🏢 Starting Call Center Server");
    info!("📞 Support Line: sip:support@{}", args.domain);
    info!("🔗 SIP Address: {}", args.bind_addr);
    info!("🌐 Public Domain: {}", args.domain);
    info!("💾 Database: {}", args.database_path);

    // Parse and validate bind address
    let bind_addr: SocketAddr = args.bind_addr.parse()
        .map_err(|e| format!("Invalid bind address '{}': {}", args.bind_addr, e))?;

    // Step 1: Configure the call center
    let mut config = CallCenterConfig::default();
    config.general.local_signaling_addr = bind_addr;
    config.general.domain = args.domain.clone();
    config.agents.default_max_concurrent_calls = args.max_calls_per_agent;
    
    // Set queue parameters
    config.queues.default_max_wait_time = args.max_wait_time;
    config.queues.max_queue_size = args.max_queue_size;

    info!("⚙️  Server configuration:");
    info!("   Domain: {}", config.general.domain);
    info!("   Bind Address: {}", config.general.local_signaling_addr);
    info!("   Max calls per agent: {}", config.agents.default_max_concurrent_calls);
    info!("   Queue max wait time: {}s", config.queues.default_max_wait_time);
    info!("   Max queue size: {}", config.queues.max_queue_size);

    // Step 2: Create the call center server
    let mut server = CallCenterServerBuilder::new()
        .with_config(config)
        .with_database_path(args.database_path.clone())
        .build()
        .await
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;

    info!("✅ Call center server created with database: {}", args.database_path);

    // Step 3: Start the server
    server.start().await
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;
    info!("🚀 Call center server started successfully");

    // Step 4: Create default queues and wait for agents to register
    server.create_default_queues().await
        .map_err(|e| Box::new(e) as Box<dyn std::error::Error>)?;
    info!("📋 Default call queues created");

    info!("👥 Waiting for agents to register...");
    info!("📞 Ready to accept customer calls on sip:support@{}", args.domain);
    info!("🎯 CALL CENTER IS READY");

    // Step 5: Start the server in the background
    // The server handles all SIP signaling, agent registration, and call routing
    let server_handle = tokio::spawn(async move {
        match server.run().await {
            Ok(_) => info!("🏁 Server completed successfully"),
            Err(e) => {
                error!("❌ Server error: {}", e);
            }
        }
    });

    // Step 6: Keep the server running until Ctrl+C
    info!("📡 Server is now running and listening for SIP traffic...");
    info!("   - Agent registrations: sip:REGISTER");
    info!("   - Customer calls: sip:support@{}", args.domain);
    info!("   - Press Ctrl+C to shutdown");
    
    // Wait for shutdown signal
    tokio::signal::ctrl_c().await?;
    
    // Cleanup
    info!("🔚 Shutting down call center server...");
    server_handle.abort();
    info!("👋 Call center server shutdown complete");

    Ok(())
} 