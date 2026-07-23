//! ICE caller (Alice) — calls Bob with `Config::enable_ice = true` and
//! prints the RFC 8445 connectivity check result.
//!
//! Run standalone:  cargo run -p rvoip-sip --example stream_peer_ice_alice --features ice
//! Or with bob:      ./examples/stream_peer/07_ice/run.sh

use rvoip_sip::{Config, Event, StreamPeer};
use tokio::time::Duration;

fn env_u16(k: &str, default: u16) -> u16 {
    std::env::var(k)
        .ok()
        .and_then(|s| s.parse().ok())
        .unwrap_or(default)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "warn,rvoip_sip_dialog=error".into()),
        )
        .init();

    let alice_port = env_u16("ALICE_SIP_PORT", 5060);
    let bob_port = env_u16("BOB_SIP_PORT", 5061);
    let media_start = env_u16("ALICE_MEDIA_PORT_START", 10000);
    let media_end = env_u16("ALICE_MEDIA_PORT_END", 10100);

    let mut alice = StreamPeer::with_config(Config {
        media_port_start: media_start,
        media_port_end: media_end,
        enable_ice: true,
        ..Config::local("alice", alice_port)
    })
    .await?;

    // Subscribe before placing the call so an IceConnected fired right
    // after the answer isn't missed.
    let mut events = alice.coordinator().events().await?;

    println!("[ALICE] Calling Bob on port {}...", bob_port);
    let call_id = alice
        .invite(format!("sip:bob@127.0.0.1:{}", bob_port))
        .send()
        .await?;
    let handle = alice.coordinator().session(&call_id);
    alice.wait_for_answered(handle.id()).await?;
    println!("[ALICE] Connected!");

    println!("[ALICE] Waiting for ICE connectivity check...");
    loop {
        match tokio::time::timeout(Duration::from_secs(10), events.next()).await {
            Ok(Some(Event::IceConnected { selected_addr, .. })) => {
                println!("[ALICE] ICE connected — selected pair remote address: {selected_addr}");
                break;
            }
            Ok(Some(_)) => continue,
            Ok(None) => {
                println!("[ALICE] Event stream closed before ICE connected");
                break;
            }
            Err(_) => {
                println!("[ALICE] Timed out waiting for IceConnected");
                break;
            }
        }
    }

    tokio::time::sleep(Duration::from_secs(1)).await;

    println!("[ALICE] Hanging up...");
    handle.hangup().await?;
    alice.wait_for_ended(handle.id()).await?;
    println!("[ALICE] Done.");

    std::process::exit(0);
}
