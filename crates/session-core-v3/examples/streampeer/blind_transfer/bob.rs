//! Transferor (Bob) — accepts call from Alice, then transfers her to Charlie.
//!
//! Run standalone:  cargo run -p rvoip-session-core-v3 --example streampeer_blind_transfer_bob
//! Or with others:  ./examples/streampeer/blind_transfer/run.sh

use rvoip_session_core_v3::{Config, StreamPeer};
use tokio::time::{sleep, Duration};

fn env_port(key: &str, default: u16) -> u16 {
    std::env::var(key).ok().and_then(|s| s.parse().ok()).unwrap_or(default)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "warn,rvoip_dialog_core=error".into()))
        .init();

    let bob_port = env_port("BOB_PORT", 5061);
    let charlie_port = env_port("CHARLIE_PORT", 5062);

    let mut bob = StreamPeer::with_config(Config::local("bob", bob_port)).await?;
    println!("[BOB] Waiting for call...");

    let incoming = bob.wait_for_incoming().await?;
    println!("[BOB] Call from {}", incoming.from);
    let handle = incoming.accept().await?;

    // Talk for 2 seconds, then transfer
    sleep(Duration::from_secs(2)).await;
    println!("[BOB] Transferring Alice to Charlie...");
    handle.transfer_blind(&format!("sip:charlie@127.0.0.1:{}", charlie_port)).await?;

    sleep(Duration::from_secs(1)).await;
    handle.hangup().await?;
    bob.wait_for_ended(handle.id()).await?;
    println!("[BOB] Done.");

    std::process::exit(0);
}
