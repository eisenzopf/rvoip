//! INTERFACE_DESIGN.md §16.2 — SIP ↔ WebRTC bridge sketch.
//!
//! Cargo features: `[sip, webrtc]`. The "why use rvoip vs.
//! FreeSWITCH + Janus + glue" demo. An inbound SIP call (PSTN) is
//! routed to a WebRTC agent in a browser; rvoip-media inserts the
//! G.711 ↔ Opus transcoder automatically.
//!
//! Run with:
//!
//! ```sh
//! cargo run --example sip_webrtc_bridge \
//!   --features sip,webrtc --manifest-path crates/rvoip/Cargo.toml
//! ```
//!
//! This is a *sketch* — the per-substrate adapter constructors and
//! routing decisions are stubbed where real configuration is needed.
//! Treat as compileable shape, not as a turnkey gateway.

use rvoip::{Config, Orchestrator};
use tracing::Level;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().with_max_level(Level::INFO).init();

    let orchestrator = Orchestrator::new(Config::default());
    let mut events = orchestrator.subscribe_events();

    tracing::info!("rvoip SIP↔WebRTC bridge sketch running");
    tracing::info!(
        "register SipAdapter (rvoip::sip) and WebRtcAdapter (rvoip::webrtc) here \
         per your tenant config"
    );

    // Pump events for ~2s then exit. A real gateway loops until SIGINT.
    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(std::time::Duration::from_millis(200), events.recv()).await {
            Ok(Ok(event)) => {
                tracing::info!(?event, "orchestrator event");
                // Real bridge logic: on Event::ConnectionInbound (SIP),
                // originate a WebRtc Connection toward the matching
                // agent's reachability hint, then call
                // orchestrator.bridge_connections(sip_conn, webrtc_conn).
            }
            _ => continue,
        }
    }

    tracing::info!("exiting (sketch only)");
    Ok(())
}
