//! Secure call (DTLS-SRTP) — the **client**.
//!
//! `Config::offer_srtp = true` plus `Config::srtp_keying =
//! SrtpKeyingMode::DtlsSrtp` makes the media adapter produce an SDP offer
//! with `m=audio … UDP/TLS/RTP/SAVP …` plus `a=fingerprint`/`a=setup`
//! (RFC 5763/5764) instead of SDES's `a=crypto:` lines. The call is
//! answered as soon as signaling completes; the actual DTLS 1.2 handshake
//! (and therefore SRTP key installation) runs in the background
//! afterward, so this demo waits for `Event::MediaSecurityNegotiated`
//! separately rather than assuming it's done the moment the call answers.
//!
//! Run with `./run_demo.sh`, or pair manually with the `server` binary.

use rvoip_sip::{Config, Event, StreamPeer};
use tokio::time::{sleep, timeout, Duration};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt()
        .with_env_filter(
            std::env::var("RUST_LOG").unwrap_or_else(|_| "warn,rvoip_sip_dialog=error".into()),
        )
        .init();

    let mut config = Config::local("dtls_client", 5062);
    config.offer_srtp = true;
    config.srtp_keying = rvoip_sip::api::unified::SrtpKeyingMode::DtlsSrtp;

    let mut peer = StreamPeer::with_config(config).await?;

    // Subscribe before placing the call so a MediaSecurityNegotiated
    // fired right after the handshake completes isn't missed.
    let mut events = peer.coordinator().events().await?;

    println!("Placing DTLS-SRTP call to sip:server@127.0.0.1:5060…");
    let call_id = peer.invite("sip:server@127.0.0.1:5060").send().await?;
    let handle = peer.coordinator().session(&call_id);
    peer.wait_for_answered(handle.id()).await?;
    println!("Call answered — waiting for the DTLS-SRTP handshake to complete…");

    let security = timeout(Duration::from_secs(10), async {
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
            "media secured — keying={keying:?} suite={suite:?} profile={profile:?} \
             contexts_installed={contexts_installed}"
        ),
        _ => println!("no MediaSecurityNegotiated event observed"),
    }

    sleep(Duration::from_millis(500)).await;

    handle.hangup().await?;
    peer.wait_for_ended(handle.id()).await?;
    println!("DTLS-SRTP call done.");

    std::process::exit(0);
}
