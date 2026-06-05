//! Real cross-transport bridge: WHIP WebRTC ingest → orchestrator → UCTP/QUIC leg.
//!
//! Pre-provisions a real `rvoip-quic::UctpQuicAdapter` connection (auth +
//! session.invite). When a WHIP publisher connects, the orchestrator bridges
//! WebRTC media to that QUIC leg.
//!
//! ```bash
//! cargo run -p rvoip-webrtc --example webrtc_quic_bridge_demo --features bridge-quic
//! ```

#[path = "../tests/support/quic_leg.rs"]
mod quic_leg;

use std::collections::HashMap;
use std::sync::Arc;

use quic_leg::{install_crypto_provider, QuicLegHarness};
use rvoip_core::adapter::ConnectionAdapter;
use rvoip_core::commands::InboundAction;
use rvoip_core::config::Config;
use rvoip_core::events::Event;
use rvoip_core::ids::{ConnectionId, ParticipantId, SessionId};
use rvoip_core::orchestrator::Orchestrator;
use rvoip_webrtc::{WebRtcConfig, WebRtcServerBuilder};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();
    install_crypto_provider();

    let whip_bind = std::env::var("WHIP_BIND").unwrap_or_else(|_| "127.0.0.1:8080".into());
    let quic_bind = std::env::var("QUIC_BIND").unwrap_or_else(|_| "127.0.0.1:4433".into());

    let quic = QuicLegHarness::start(quic_bind.parse()?).await;
    let server = WebRtcServerBuilder::new(WebRtcConfig::default())
        .with_whip(&whip_bind)
        .build()
        .await?;

    let orchestrator = Orchestrator::new(Config::default());
    orchestrator.register(server.adapter() as Arc<dyn ConnectionAdapter>)?;
    orchestrator.register(quic.adapter.clone() as Arc<dyn ConnectionAdapter>)?;

    tracing::info!(quic_addr = %quic.server_addr, "QUIC/UCTP listening");
    tracing::info!(?whip_bind, whip_addr = ?server.whip_addr(), "WHIP listening");

    // Stand up the QUIC leg before WHIP traffic arrives.
    let session_id = SessionId::new();
    quic.dial_invite(&session_id.to_string(), "bridge_peer")
        .await;

    let mut events = orchestrator.subscribe_events();
    let mut quic_conn: Option<ConnectionId> = None;
    let mut pending_webrtc: HashMap<ConnectionId, SessionId> = HashMap::new();

    loop {
        let event = events.recv().await?;
        match event {
            Event::ConnectionInbound { connection_id, .. } => {
                if quic_conn.is_none() && !server.adapter().routes().contains_key(&connection_id) {
                    tracing::info!(%connection_id, "QUIC/UCTP leg ready");
                    quic_conn = Some(connection_id);
                    continue;
                }

                if server.adapter().routes().contains_key(&connection_id) {
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
                }
            }
            Event::ConnectionConnected { connection_id, .. } => {
                if let Some(sid) = pending_webrtc.remove(&connection_id) {
                    let Some(quic_id) = quic_conn.clone() else {
                        tracing::error!("WebRTC connected before QUIC leg was ready");
                        continue;
                    };
                    tracing::info!(%connection_id, %sid, %quic_id, "bridging WebRTC → QUIC");
                    match orchestrator
                        .bridge_connections(connection_id.clone(), quic_id)
                        .await
                    {
                        Ok(bridge_id) => tracing::info!(%bridge_id, "bridge active"),
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
