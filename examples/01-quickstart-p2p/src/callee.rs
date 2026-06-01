//! Quickstart P2P — the **callee** (Bob).
//!
//! Listens for one inbound call, auto-answers it, and waits for the caller to
//! hang up before shutting down.
//!
//! Run both sides together with `./run_demo.sh`, or manually:
//!
//! ```text
//! cargo run --bin callee -- --port 5061      # terminal 1
//! cargo run --bin caller -- --peer-port 5061 # terminal 2
//! ```

use clap::Parser;
use rvoip_sip::{Config, Result, StreamPeer};

#[derive(Parser, Debug)]
#[command(about = "Quickstart P2P callee — waits for a call, answers, waits for hangup")]
struct Args {
    /// SIP port this callee binds to.
    #[arg(long, default_value_t = 5061)]
    port: u16,
}

#[tokio::main]
async fn main() -> Result<()> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "warn".into()))
        .init();
    let args = Args::parse();

    let mut bob = StreamPeer::with_config(Config::local("callee", args.port)).await?;
    println!("[callee] listening on sip:callee@127.0.0.1:{}", args.port);

    // Block until an INVITE arrives, then accept it (sends 200 OK + SDP).
    let incoming = bob.wait_for_incoming().await?;
    println!("[callee] incoming call from {}", incoming.from);
    let call = incoming.accept().await?;
    println!("[callee] ✅ answered {}", call.id());

    // Stay up until the caller sends BYE.
    call.wait_for_end(None).await?;
    println!("[callee] ✅ call ended");
    bob.shutdown().await
}
