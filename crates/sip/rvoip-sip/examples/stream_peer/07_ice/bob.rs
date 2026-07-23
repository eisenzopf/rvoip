//! ICE answerer (Bob) — accepts a call with `Config::enable_ice = true`
//! and prints the RFC 8445 connectivity check result.
//!
//! Run standalone:  cargo run -p rvoip-sip --example stream_peer_ice_bob --features ice
//! Or with alice:    ./examples/stream_peer/07_ice/run.sh

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

    let bob_port = env_u16("BOB_SIP_PORT", 5061);
    let media_start = env_u16("BOB_MEDIA_PORT_START", 10100);
    let media_end = env_u16("BOB_MEDIA_PORT_END", 10200);

    let mut bob = StreamPeer::with_config(Config {
        media_port_start: media_start,
        media_port_end: media_end,
        enable_ice: true,
        ..Config::local("bob", bob_port)
    })
    .await?;

    // Subscribe before accepting so an IceConnected fired right after
    // the 200 OK isn't missed.
    let mut events = bob.coordinator().events().await?;

    println!("[BOB] Waiting for call...");
    let incoming = bob.wait_for_incoming().await?;
    println!("[BOB] Call from {}", incoming.from);
    let handle = incoming.accept().await?;

    println!("[BOB] Waiting for ICE connectivity check...");
    loop {
        match tokio::time::timeout(Duration::from_secs(10), events.next()).await {
            Ok(Some(Event::IceConnected { selected_addr, .. })) => {
                println!("[BOB] ICE connected — selected pair remote address: {selected_addr}");
                break;
            }
            Ok(Some(_)) => continue,
            Ok(None) => {
                println!("[BOB] Event stream closed before ICE connected");
                break;
            }
            Err(_) => {
                println!("[BOB] Timed out waiting for IceConnected");
                break;
            }
        }
    }

    handle
        .wait_for_end(Some(Duration::from_secs(10)))
        .await
        .ok();
    println!("[BOB] Done.");

    std::process::exit(0);
}
