//! Bridge demo: WHIP WebRTC ingest → orchestrator → synthetic QUIC leg.
//!
//! For the real `rvoip-quic` leg see `webrtc_quic_bridge_demo` (`bridge-quic` feature).
//!
//! ```bash
//! cargo run -p rvoip-webrtc --example webrtc_bridge_demo --features signaling-whip
//! ```

#[path = "../tests/support/mock_quic_leg.rs"]
mod mock_quic_leg;

use std::collections::HashMap;
use std::sync::Arc;

use mock_quic_leg::MockQuicLeg;
use rvoip_core::adapter::ConnectionAdapter;
use rvoip_core::commands::InboundAction;
use rvoip_core::config::Config;
use rvoip_core::connection::Transport;
use rvoip_core::events::Event;
use rvoip_core::ids::{ConnectionId, ParticipantId, SessionId};
use rvoip_core::orchestrator::Orchestrator;
use rvoip_webrtc::{WebRtcConfig, WebRtcServerBuilder};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();
    let _ = rustls::crypto::ring::default_provider().install_default();

    let whip_bind = std::env::var("WHIP_BIND").unwrap_or_else(|_| "127.0.0.1:8080".into());

    let server = WebRtcServerBuilder::new(WebRtcConfig::default())
        .with_whip(&whip_bind)
        .build()
        .await?;

    let quic_leg = MockQuicLeg::new();
    let session_id = SessionId::new();
    let (quic_conn, _quic_stream) = quic_leg.provision_inbound(session_id, "opus").await;

    let orchestrator = Orchestrator::new(Config::default());
    orchestrator.register(server.adapter() as Arc<dyn ConnectionAdapter>)?;
    orchestrator.register(quic_leg as Arc<dyn ConnectionAdapter>)?;

    tracing::info!(?whip_bind, addr = ?server.whip_addr(), "WHIP listening (publish here)");
    tracing::info!(%quic_conn, "synthetic QUIC leg pre-provisioned — will bridge on WebRTC connect");

    let mut events = orchestrator.subscribe_events();
    let mut pending_webrtc: HashMap<ConnectionId, SessionId> = HashMap::new();

    loop {
        let event = events.recv().await?;
        match event {
            Event::ConnectionInbound { connection_id, .. } => {
                // Distinguish WebRTC (WHIP) from the pre-provisioned QUIC mock.
                if orchestrator.adapter(Transport::WebRtc).is_ok()
                    && server.adapter().routes().contains_key(&connection_id)
                {
                    tracing::info!(%connection_id, "WHIP publish — accepting WebRTC leg");
                    let sid = SessionId::new();
                    pending_webrtc.insert(connection_id.clone(), sid.clone());
                    orchestrator
                        .route_inbound_connection(
                            connection_id,
                            InboundAction::Accept {
                                session_id: sid,
                                participant_id: ParticipantId::new(),
                            },
                        )
                        .await?;
                } else {
                    tracing::debug!(%connection_id, "QUIC mock leg registered");
                }
            }
            Event::ConnectionConnected { connection_id, .. } => {
                if let Some(sid) = pending_webrtc.remove(&connection_id) {
                    tracing::info!(%connection_id, %sid, "WebRTC connected — bridging to QUIC leg");
                    match orchestrator
                        .bridge_connections(connection_id.clone(), quic_conn.clone())
                        .await
                    {
                        Ok(bridge_id) => {
                            tracing::info!(%bridge_id, "bridge active — media pumps running");
                        }
                        Err(e) => tracing::error!(%connection_id, "bridge failed: {e}"),
                    }
                }
            }
            Event::ConnectionsBridged {
                bridge_id, a, b, ..
            } => {
                tracing::info!(%bridge_id, %a, %b, "ConnectionsBridged");
            }
            Event::ConnectionEnded { connection_id, .. } => {
                tracing::info!(%connection_id, "connection ended");
            }
            Event::ConnectionFailed { connection_id, .. } => {
                tracing::warn!(%connection_id, "connection failed");
            }
            other => tracing::debug!(?other, "event"),
        }
    }
}
