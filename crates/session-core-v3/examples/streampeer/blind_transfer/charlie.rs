//! Transfer target (Charlie) — waits for the transferred call from Alice.
//!
//! Run standalone:  cargo run -p rvoip-session-core-v3 --example streampeer_blind_transfer_charlie
//! Or with others:  ./examples/streampeer/blind_transfer/run.sh

use rvoip_session_core_v3::{Config, StreamPeer};
use tokio::time::Duration;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "warn,rvoip_dialog_core=error".into()))
        .init();

    let mut charlie = StreamPeer::with_config(Config::local("charlie", 5062)).await?;
    println!("[CHARLIE] Waiting for transferred call...");

    let incoming = charlie.wait_for_incoming().await?;
    println!("[CHARLIE] Incoming call from {}", incoming.from);
    let handle = incoming.accept().await?;
    println!("[CHARLIE] Answered!");

    handle.wait_for_end(Some(Duration::from_secs(30))).await.ok();
    println!("[CHARLIE] Call ended.");

    std::process::exit(0);
}
