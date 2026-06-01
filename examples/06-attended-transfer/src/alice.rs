//! Attended transfer — **Alice** (the transferee).
//!
//! Calls Bob, then waits for the attended-transfer REFER. The REFER's Refer-To
//! carries an embedded `Replaces` pointing at Bob's consultation dialog with
//! Charlie, so Alice completes the transfer by placing a fresh INVITE to the
//! Refer-To target — which replaces that consultation leg.

use rvoip_sip::{Config, Event, StreamPeer};
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
            std::env::var("RUST_LOG").unwrap_or_else(|_| "warn,rvoip_sip_dialog=error".into()),
        )
        .init();

    let alice_port = env_port("ALICE_PORT", 5060);
    let bob_port = env_port("BOB_PORT", 5061);

    let mut alice = StreamPeer::with_config(Config::local("alice", alice_port)).await?;

    println!("[ALICE] Calling Bob...");
    let call_id = alice
        .invite(format!("sip:bob@127.0.0.1:{bob_port}"))
        .send()
        .await?;
    let handle = alice.coordinator().session(&call_id);
    alice.wait_for_answered(handle.id()).await?;
    println!("[ALICE] Connected to Bob.");

    println!("[ALICE] Waiting for attended transfer...");
    let mut events = alice.control().subscribe_events().await?;
    loop {
        match events.next().await {
            Some(Event::ReferReceived {
                refer_to, replaces, ..
            }) => {
                println!("[ALICE] Got REFER to {refer_to}");
                println!("[ALICE] Replaces = {replaces:?}");
                handle.hangup().await?;
                alice.wait_for_ended(handle.id()).await?;

                println!("[ALICE] Completing transfer — calling the Refer-To target...");
                let charlie_id = alice.invite(refer_to.clone()).send().await?;
                let charlie = alice.coordinator().session(&charlie_id);
                alice.wait_for_answered(charlie.id()).await?;
                println!("[ALICE] ✅ Connected to Charlie (attended transfer complete).");

                sleep(Duration::from_secs(1)).await;
                charlie.hangup().await?;
                alice.wait_for_ended(charlie.id()).await?;
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
