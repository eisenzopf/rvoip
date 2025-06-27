//! Main entry point for the RVOIP SIP Client CLI
//!
//! This binary provides a command-line interface for SIP operations
//! built on the RVOIP stack.

use clap::Parser;
use rvoip_sip_client::cli::Cli;

#[tokio::main]
async fn main() {
    let cli = Cli::parse();
    
    if let Err(e) = cli.execute().await {
        eprintln!("❌ Error: {}", e.user_message());
        std::process::exit(1);
    }
} 