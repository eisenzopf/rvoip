//! Attended transfer — **Charlie** (the transfer target).
//!
//! Answers Bob's consultation call first, then answers the replacing call Alice
//! places to complete the attended transfer.

use rvoip_sip::{Config, StreamPeer};
use tokio::time::Duration;

fn env_port(key: &str, default: u16) -> u16 {
    std::env::var(key)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "warn,rvoip_sip_dialog=error".into()),
        )
        .init();

    let charlie_port = env_port("CHARLIE_PORT", 5062);

    let mut charlie = StreamPeer::with_config(Config::local("charlie", charlie_port)).await?;
    println!("[CHARLIE] Waiting for consultation call from Bob...");

    let consultation = charlie.wait_for_incoming().await?;
    println!("[CHARLIE] Consultation from {}", consultation.from);
    let _consult = consultation.accept().await?;
    println!("[CHARLIE] Consultation answered.");

    println!("[CHARLIE] Waiting for the transferred (replacing) call from Alice...");
    let replacing = charlie.wait_for_incoming().await?;
    println!("[CHARLIE] Transferred call from {}", replacing.from);
    let handle = replacing.accept().await?;
    println!("[CHARLIE] ✅ Answered transferred call.");

    handle.wait_for_end(Some(Duration::from_secs(30))).await.ok();
    println!("[CHARLIE] Done.");
    std::process::exit(0);
}
