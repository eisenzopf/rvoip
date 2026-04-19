//! RFC 4028 §10 session-timer refresh FAILURE — Bob side.
//!
//! Bob accepts an incoming call then, before Alice's first refresh fires,
//! exits the process. This simulates the "remote peer crashed" scenario:
//! Alice's UPDATE lands on a dead UDP port, the transaction times out,
//! and dialog-core's session_timer path tears the dialog down with a
//! `Reason: SIP ;cause=408` BYE.

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

    let bob_port = env_port("BOB_PORT", 35074);

    let mut config = Config::local("bob", bob_port);
    config.session_timer_secs = Some(4);
    config.session_timer_min_se = 2;

    let mut bob = StreamPeer::with_config(config).await?;
    println!("[BOB] Listening on {} (session timer = 4s)", bob_port);

    let incoming = bob.wait_for_incoming().await?;
    println!("[BOB] Call from {}", incoming.from);
    let _handle = incoming.accept().await?;

    // Wait until the call is solidly established (ACK + SDP round-trip),
    // then yank the peer out from under Alice before the first refresh
    // fires at t≈2 s (half of 4 s). No graceful shutdown — this is the
    // "remote crashed" scenario. Alice's UPDATE will land on a dead port.
    sleep(Duration::from_millis(1500)).await;
    println!("[BOB] Simulating crash — exiting now.");
    std::process::exit(0);
}
