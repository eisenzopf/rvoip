//! INTERFACE_DESIGN.md §16.3 — Pure UCTP application server sketch.
//!
//! Cargo features: `[uctp, vcon, identity]`. No SIP, no WebRTC — a
//! messaging-and-voice app where mobile / web / desktop clients
//! connect over QUIC / WebTransport / WebSocket and exchange
//! Messages and Sessions with each other.
//!
//! Run with:
//!
//! ```sh
//! cargo run --example uctp_only_server \
//!   --features uctp,vcon,identity \
//!   --manifest-path crates/rvoip/Cargo.toml
//! ```
//!
//! Sketch only: the real server wires `rvoip::uctp::quic::UctpQuicAdapter`
//! / `webtransport::UctpWtAdapter` / `websocket::UctpWsAdapter`
//! against a TLS config and an `IdentityProvider` from
//! `rvoip::identity`. Routing inbound `session.invite` to a target
//! Identity uses `IdentityProvider::reachable_via` (per
//! INTERFACE_DESIGN §8.2).

use rvoip::{Config, Orchestrator};
use tracing::Level;

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt().with_max_level(Level::INFO).init();

    let orchestrator = Orchestrator::new(Config::default());
    let mut events = orchestrator.subscribe_events();

    tracing::info!(
        "rvoip UCTP-only server sketch — bind QUIC / WT / WS adapters \
         from rvoip::uctp here"
    );

    let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(2);
    while tokio::time::Instant::now() < deadline {
        match tokio::time::timeout(std::time::Duration::from_millis(200), events.recv()).await {
            Ok(Ok(event)) => {
                tracing::info!(?event, "uctp event");
                // Real server: on Event::ConversationOpened, persist via
                // ConversationStore; on Event::MessageReceived, fan-out;
                // on Event::VconReady, archive the signed vCon.
            }
            _ => continue,
        }
    }

    tracing::info!("exiting (sketch only)");
    Ok(())
}
