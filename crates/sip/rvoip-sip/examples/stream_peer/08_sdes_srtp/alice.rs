//! SDES-SRTP caller (Alice) — mandatory RFC 4568 SDES key exchange.
//!
//! `Config::offer_srtp = true` makes the media adapter offer `m=audio …
//! RTP/SAVP …` with `a=crypto:` lines; `srtp_required = true` refuses
//! plaintext fallback. `Config::srtp_keying` defaults to
//! `SrtpKeyingMode::Sdes`, so no extra config is needed to select it —
//! contrast with [09_dtls_srtp](../09_dtls_srtp/), which sets it explicitly
//! to `DtlsSrtp`.
//!
//! Run standalone:  cargo run -p rvoip-sip --example stream_peer_sdes_srtp_alice
//! Or with bob:      ./examples/stream_peer/08_sdes_srtp/run.sh

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

    let mut config = Config::local("alice", alice_port);
    config.offer_srtp = true;
    config.srtp_required = true;
    let mut alice = StreamPeer::with_config(config).await?;

    // Subscribe before placing the call so a MediaSecurityNegotiated fired
    // right after the answer isn't missed.
    let mut events = alice.coordinator().events().await?;

    println!("[alice] calling bob on port {}...", bob_port);
    let call_id = alice
        .invite(format!("sip:bob@127.0.0.1:{}", bob_port))
        .send()
        .await?;
    let handle = alice.coordinator().session(&call_id);
    alice.wait_for_answered(handle.id()).await?;
    println!("[alice] connected!");

    println!("[alice] waiting for SDES-SRTP negotiation...");
    let security = tokio::time::timeout(Duration::from_secs(10), async {
        loop {
            match events.next().await {
                Some(ev @ Event::MediaSecurityNegotiated { .. }) => return Some(ev),
                Some(_) => continue,
                None => return None,
            }
        }
    })
    .await
    .ok()
    .flatten();

    match security {
        Some(Event::MediaSecurityNegotiated {
            keying,
            suite,
            profile,
            contexts_installed,
            ..
        }) => println!(
            "[alice] media secured — keying={keying:?} suite={suite:?} profile={profile:?} \
             contexts_installed={contexts_installed}"
        ),
        _ => println!("[alice] no MediaSecurityNegotiated event observed"),
    }

    tokio::time::sleep(Duration::from_secs(1)).await;

    println!("[alice] hanging up...");
    handle.hangup().await?;
    alice.wait_for_ended(handle.id()).await?;
    println!("[alice] done.");

    std::process::exit(0);
}
