//! Caller (Alice) — calls Bob, receives REFER, then calls Charlie.
//!
//! Run standalone:  cargo run -p rvoip-session-core --example streampeer_blind_transfer_alice
//! Or with others:  ./examples/streampeer/blind_transfer/run.sh

use rvoip_session_core::{Config, Event, StreamPeer};
use tokio::time::{sleep, Duration};

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
            std::env::var("RUST_LOG").unwrap_or_else(|_| "warn,rvoip_dialog_core=error".into()),
        )
        .init();

    // Port overrides (for the integration test); defaults match run.sh.
    let alice_port = env_port("ALICE_PORT", 5060);
    let bob_port = env_port("BOB_PORT", 5061);

    let mut alice = StreamPeer::with_config(Config::local("alice", alice_port)).await?;

    println!("[ALICE] Calling Bob...");
    let handle = alice
        .call(&format!("sip:bob@127.0.0.1:{}", bob_port))
        .await?;
    alice.wait_for_answered(handle.id()).await?;
    println!("[ALICE] Connected to Bob!");

    // Wait for REFER from Bob
    println!("[ALICE] Waiting for transfer...");
    let mut events = alice.control().subscribe_events().await?;
    loop {
        match events.next().await {
            Some(Event::ReferReceived { refer_to, .. }) => {
                println!("[ALICE] Got REFER to {}", refer_to);
                // Hang up with Bob and call Charlie
                handle.hangup().await?;
                alice.wait_for_ended(handle.id()).await?;

                println!("[ALICE] Calling Charlie...");
                let charlie_handle = alice.call(&refer_to).await?;
                alice.wait_for_answered(charlie_handle.id()).await?;
                println!("[ALICE] Connected to Charlie!");

                sleep(Duration::from_secs(2)).await;
                charlie_handle.hangup().await?;
                alice.wait_for_ended(charlie_handle.id()).await?;
                break;
            }
            Some(Event::CallEnded { .. }) => {
                println!("[ALICE] Call ended before transfer");
                break;
            }
            None => break,
            _ => {}
        }
    }

    println!("[ALICE] Done.");

    std::process::exit(0);
}
