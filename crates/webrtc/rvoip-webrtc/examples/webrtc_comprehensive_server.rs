//! Comprehensive WebRTC server — WS signaling + auto-accept + media/DC handlers.

use std::path::PathBuf;
use std::sync::Arc;

use rvoip_core::adapter::ConnectionAdapter;
use rvoip_core::commands::InboundAction;
use rvoip_core::config::Config;
use rvoip_core::events::Event;
use rvoip_core::ids::{ParticipantId, SessionId};
use rvoip_core::orchestrator::Orchestrator;
use rvoip_webrtc::client::comprehensive::handle_server_connection;
use rvoip_webrtc::{WebRtcConfig, WebRtcServerBuilder};

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();
    let _ = rustls::crypto::ring::default_provider().install_default();

    let ws_bind = std::env::var("WS_BIND").unwrap_or_else(|_| "127.0.0.1:8081".into());
    let ready_file = std::env::var("READY_FILE").ok().map(PathBuf::from);

    let server = WebRtcServerBuilder::new(WebRtcConfig::loopback())
        .with_ws(&ws_bind)
        .build()
        .await?;

    let ws_addr = server.ws_addr().expect("websocket listener address");

    if let Some(path) = &ready_file {
        std::fs::write(path, ws_addr.to_string())?;
        tracing::info!(?path, %ws_addr, "wrote ready file");
    }

    let orchestrator = Arc::new(Orchestrator::new(Config::default()));
    orchestrator.register(server.adapter() as Arc<dyn ConnectionAdapter>)?;

    tracing::info!(%ws_addr, "comprehensive WebRTC server listening");

    let adapter = server.adapter().clone();
    let mut events = orchestrator.subscribe_events();
    loop {
        let event = events.recv().await?;
        match event {
            Event::ConnectionInbound { connection_id, .. } => {
                if adapter.routes().contains_key(&connection_id) {
                    tracing::info!(%connection_id, "inbound offer — accepting");
                    let adapter_spawn = Arc::clone(&adapter);
                    let conn_spawn = connection_id.clone();
                    tokio::spawn(async move {
                        handle_server_connection(adapter_spawn, conn_spawn).await;
                    });
                    orchestrator
                        .route_inbound_connection(
                            connection_id.clone(),
                            InboundAction::Accept {
                                session_id: SessionId::new(),
                                participant_id: ParticipantId::new(),
                            },
                        )
                        .await?;
                }
            }
            Event::ConnectionConnected { connection_id, .. } => {
                tracing::info!(%connection_id, "connected");
            }
            Event::ConnectionEnded { connection_id, .. } => {
                tracing::info!(%connection_id, "ended");
            }
            other => tracing::debug!(?other, "event"),
        }
    }
}
