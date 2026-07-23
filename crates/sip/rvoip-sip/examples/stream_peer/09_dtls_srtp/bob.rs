//! DTLS-SRTP answerer (Bob) — RFC 5763/5764 key exchange.
//!
//! Requires the `dtls-srtp` feature.
//!
//! Run standalone:  cargo run -p rvoip-sip --example stream_peer_dtls_srtp_bob --features dtls-srtp
//! Or with alice:    ./examples/stream_peer/09_dtls_srtp/run.sh

use rvoip_sip::api::unified::SrtpKeyingMode;
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

    let mut config = Config::local("bob", bob_port);
    config.offer_srtp = true;
    config.srtp_keying = SrtpKeyingMode::DtlsSrtp;
    let mut bob = StreamPeer::with_config(config).await?;

    // Subscribe before accepting so a MediaSecurityNegotiated fired right
    // after the DTLS handshake completes isn't missed.
    let mut events = bob.coordinator().events().await?;

    println!("[bob] waiting for call...");
    let incoming = bob.wait_for_incoming().await?;
    println!("[bob] call from {}", incoming.from);
    let handle = incoming.accept().await?;

    println!("[bob] waiting for the DTLS-SRTP handshake to complete...");
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
            "[bob] media secured — keying={keying:?} suite={suite:?} profile={profile:?} \
             contexts_installed={contexts_installed}"
        ),
        _ => println!("[bob] no MediaSecurityNegotiated event observed"),
    }

    handle
        .wait_for_end(Some(Duration::from_secs(10)))
        .await
        .ok();
    println!("[bob] done.");

    std::process::exit(0);
}
