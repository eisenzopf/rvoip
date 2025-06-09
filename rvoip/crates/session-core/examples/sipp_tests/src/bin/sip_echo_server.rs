//! SIP Echo Server - Advanced test server for audio verification

use anyhow::Result;
use clap::Parser;
use tracing::info;

#[derive(Parser, Debug)]
#[command(name = "sip_echo_server")]
#[command(about = "SIP Echo Server for audio testing")]
pub struct Args {
    /// SIP listening port
    #[arg(short, long, default_value = "5063")]
    pub port: u16,
    
    /// Echo delay in milliseconds
    #[arg(short, long, default_value = "100")]
    pub delay: u64,
    
    /// Log level
    #[arg(short, long, default_value = "info")]
    pub log_level: String,
}

#[tokio::main]
async fn main() -> Result<()> {
    let args = Args::parse();
    
    // Initialize tracing
    tracing_subscriber::fmt()
        .with_env_filter(&args.log_level)
        .with_target(false)
        .init();
    
    info!("🧪 SIP Echo Server starting...");
    info!("📡 Port: {}", args.port);
    info!("⏰ Echo delay: {}ms", args.delay);
    
    // TODO: Implement SIP echo server functionality
    info!("🔧 [STUB] SIP Echo Server functionality will be implemented here");
    
    Ok(())
} 