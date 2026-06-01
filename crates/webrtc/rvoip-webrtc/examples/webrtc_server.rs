//! Dual-role WebRTC server: WHIP/WHEP + WS signaling on one adapter, registered with the orchestrator.
//!
//! ```bash
//! cargo run -p rvoip-webrtc --example webrtc_server \
//!   --features signaling-whip,signaling-ws
//! ```
//!
//! Environment:
//! - `WHIP_BIND` — default `127.0.0.1:8080`
//! - `WS_BIND`   — default `127.0.0.1:8081`

use std::sync::Arc;

use rvoip_core::adapter::ConnectionAdapter;
use rvoip_core::commands::InboundAction;
use rvoip_core::config::Config;
use rvoip_core::events::Event;
use rvoip_core::ids::{ParticipantId, SessionId};
use rvoip_core::orchestrator::Orchestrator;
use rvoip_webrtc::{WebRtcConfig, WebRtcServerBuilder};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();
    let _ = rustls::crypto::ring::default_provider().install_default();

    let whip_bind = std::env::var("WHIP_BIND").unwrap_or_else(|_| "127.0.0.1:8080".into());
    let ws_bind = std::env::var("WS_BIND").unwrap_or_else(|_| "127.0.0.1:8081".into());

    let server = WebRtcServerBuilder::new(WebRtcConfig::default())
        .with_whip(&whip_bind)
        .with_ws(&ws_bind)
        .build()
        .await?;

    let orchestrator = Orchestrator::new(Config::default());
    orchestrator.register(server.adapter() as Arc<dyn ConnectionAdapter>)?;

    tracing::info!(?whip_bind, addr = ?server.whip_addr(), "WHIP/WHEP listening");
    tracing::info!(?ws_bind, addr = ?server.ws_addr(), "WebSocket signaling listening");

    let mut events = orchestrator.subscribe_events();
    loop {
        let event = events.recv().await?;
        match event {
            Event::ConnectionInbound { connection_id, .. } => {
                tracing::info!(%connection_id, "inbound WebRTC connection — accepting");
                orchestrator
                    .route_inbound_connection(
                        connection_id,
                        InboundAction::Accept {
                            session_id: SessionId::new(),
                            participant_id: ParticipantId::new(),
                        },
                    )
                    .await?;
            }
            Event::ConnectionConnected { connection_id, .. } => {
                tracing::info!(%connection_id, "WebRTC connection up");
            }
            Event::ConnectionEnded { connection_id, .. } => {
                tracing::info!(%connection_id, "WebRTC connection ended");
            }
            Event::ConnectionFailed { connection_id, .. } => {
                tracing::warn!(%connection_id, "WebRTC connection failed");
            }
            other => {
                tracing::debug!(?other, "orchestrator event");
            }
        }
    }
}
