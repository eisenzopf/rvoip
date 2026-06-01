//! Secure call (SDES-SRTP) — the **client**.
//!
//! `Config::offer_srtp = true` makes the media adapter produce an SDP offer
//! with `m=audio … RTP/SAVP …` plus two `a=crypto:` lines
//! (AES-CM-128/HMAC-SHA1-80 preferred, -32 fallback) per RFC 4568. With
//! `srtp_required = true` the call aborts if the peer answers without
//! accepting an offered suite — the "SDES-only, no plaintext fallback"
//! posture carriers configure as `srtp=mandatory`.
//!
//! Run with `./run_demo.sh`, or pair manually with the `server` binary.

use rvoip_sip::{Config, StreamPeer};
use tokio::time::{sleep, Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "warn,rvoip_sip_dialog=error".into()),
        )
        .init();

    let mut config = Config::local("srtp_client", 5062);
    config.offer_srtp = true;
    config.srtp_required = true;

    let mut peer = StreamPeer::with_config(config).await?;

    println!("Placing SRTP-mandatory call to sip:server@127.0.0.1:5060…");
    let call_id = peer.invite("sip:server@127.0.0.1:5060").send().await?;
    let handle = peer.coordinator().session(&call_id);
    peer.wait_for_answered(handle.id()).await?;
    println!("✅ Call answered — SRTP negotiation completed, media is encrypted.");

    sleep(Duration::from_millis(500)).await;

    handle.hangup().await?;
    peer.wait_for_ended(handle.id()).await?;
    println!("SRTP call done.");

    std::process::exit(0);
}
