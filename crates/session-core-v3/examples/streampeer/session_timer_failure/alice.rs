//! RFC 4028 §10 session-timer refresh FAILURE — Alice side.
//!
//! Alice calls Bob with a short (4 s) Session-Expires and expects an
//! `Event::SessionRefreshFailed` within 15 s. Bob is configured to accept
//! the call and then exit the process before the refresh fires — Alice's
//! UPDATE lands on a dead peer and the transaction times out; the fallback
//! re-INVITE also fails; dialog-core then sends BYE with
//! `Reason: SIP ;cause=408 ;text="Session expired"` and surfaces
//! `SessionRefreshFailed` up to the session layer.
//!
//! Exits 0 iff the failure event arrives within 15 s.

use rvoip_session_core_v3::{Config, Event, StreamPeer};
use tokio::time::{timeout, Duration};

fn env_port(key: &str, default: u16) -> u16 {
    std::env::var(key).ok().and_then(|s| s.parse().ok()).unwrap_or(default)
}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(std::env::var("RUST_LOG").unwrap_or_else(|_| "warn,rvoip_dialog_core=error".into()))
        .init();

    let alice_port = env_port("ALICE_PORT", 35073);
    let bob_port = env_port("BOB_PORT", 35074);

    // Short session timer: refresh fires at t≈2 s (half of 4 s).
    // Bob exits at t=3 s → UPDATE has no one to talk to.
    let mut config = Config::local("alice", alice_port);
    config.session_timer_secs = Some(4);
    config.session_timer_min_se = 2;

    let mut alice = StreamPeer::with_config(config).await?;
    let mut events = alice.control().subscribe_events().await?;

    println!("[ALICE] Calling Bob (session timer = 4s, min-SE = 2s)…");
    let handle = alice.call(&format!("sip:bob@127.0.0.1:{}", bob_port)).await?;
    alice.wait_for_answered(handle.id()).await?;
    println!("[ALICE] Connected. Waiting for SessionRefreshFailed…");

    let outcome = timeout(Duration::from_secs(15), async {
        loop {
            match events.next().await {
                Some(Event::SessionRefreshFailed { reason, .. }) => {
                    return Some(reason);
                }
                Some(Event::SessionRefreshed { expires_secs, .. }) => {
                    eprintln!(
                        "[ALICE] Unexpected SessionRefreshed ({}s) — peer was supposed to be dead",
                        expires_secs
                    );
                    return None;
                }
                Some(_) => continue,
                None => return None,
            }
        }
    })
    .await;

    match outcome {
        Ok(Some(reason)) => {
            println!("[ALICE] Observed SessionRefreshFailed: {}", reason);
            std::process::exit(0);
        }
        _ => {
            eprintln!("[ALICE] Did not observe SessionRefreshFailed within 15s");
            std::process::exit(1);
        }
    }
}
