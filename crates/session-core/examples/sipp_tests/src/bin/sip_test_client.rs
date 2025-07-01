//! SIP Test Client - UAC that makes calls to SIPp UAS scenarios

use anyhow::Result;
use clap::Parser;
use tracing::info;

#[derive(Parser, Debug)]
#[command(name = "sip_test_client")]
#[command(about = "SIP Test Client for SIPp integration testing")]
pub struct Args {
    /// Target SIP server
    #[arg(short, long, default_value = "127.0.0.1:5060")]
    pub target: String,
    
    /// Number of calls to make
    #[arg(short, long, default_value = "1")]
    pub calls: u32,
    
    /// Call rate (calls per second)
    #[arg(short, long, default_value = "1.0")]
    pub rate: f64,
    
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
    
    info!("🧪 SIP Test Client starting...");
    info!("🎯 Target: {}", args.target);
    info!("📞 Calls: {}", args.calls);
    info!("⚡ Rate: {} calls/sec", args.rate);
    
    // TODO: Implement SIP client functionality
    info!("🔧 [STUB] SIP Test Client functionality will be implemented here");
    
    Ok(())
} 