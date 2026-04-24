//! SRTP SIP client — places an AES-128 encrypted call.
//!
//! `Config::offer_srtp = true` causes the media adapter to produce an
//! offer with `m=audio … RTP/SAVP …` + two `a=crypto:` lines
//! (AES-CM-128/HMAC-SHA1-80 preferred, -32 fallback) per RFC 4568
//! §6.2.1. With `srtp_required = true` the call aborts if the peer
//! answers without accepting any offered suite — this mirrors the
//! "SDES-only, no plaintext" profile most carriers want.
//!
//! Run standalone:  cargo run -p rvoip-session-core --example streampeer_srtp_client
//! Or with server:  ./examples/streampeer/srtp/run.sh

use rvoip_session_core::{Config, StreamPeer};
use tokio::time::{sleep, Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "warn,rvoip_dialog_core=error".into()),
        )
        .init();

    let mut config = Config::local("srtp_client", 5062);
    config.offer_srtp = true;
    config.srtp_required = true;

    let mut peer = StreamPeer::with_config(config).await?;

    println!("Placing SRTP-mandatory call to sip:server@127.0.0.1:5060…");
    let handle = peer.call("sip:server@127.0.0.1:5060").await?;
    peer.wait_for_answered(handle.id()).await?;
    println!("Call answered — SRTP negotiation completed, media is encrypted.");

    sleep(Duration::from_millis(500)).await;

    handle.hangup().await?;
    peer.wait_for_ended(handle.id()).await?;
    println!("SRTP call done.");

    std::process::exit(0);
}
