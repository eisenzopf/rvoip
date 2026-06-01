//! Browser-driven RFC validation demo.
//!
//! Boots WHIP/WHEP + WS signaling + a static HTTP server in one process so
//! Playwright (or a human in Chrome) can exercise the full WebRTC surface:
//! RFC 9725 (WHIP/WHEP HTTP), RFC 8840 (trickle PATCH), RFC 8825/8826/8829
//! (JSEP offer/answer + bidirectional A+V), RFC 8831/8832 (data channels),
//! RFC 8445/8839 (ICE candidates + mDNS filter).
//!
//! ```bash
//! cargo run -p rvoip-webrtc --example webrtc_browser_demo \
//!   --features comprehensive,signaling-whip,signaling-ws
//! ```
//!
//! On boot prints exactly one machine-readable line to stdout for the
//! Playwright `beforeAll`:
//!
//! ```text
//! [webrtc_browser_demo] READY whip=http://127.0.0.1:PORT ws=ws://127.0.0.1:PORT static=http://127.0.0.1:PORT
//! ```

use std::collections::HashSet;
use std::path::PathBuf;
use std::sync::Arc;
use std::time::Duration;

use axum::{
    extract::Path,
    http::{HeaderMap, HeaderValue, StatusCode},
    response::IntoResponse,
    routing::get,
    Router,
};
use parking_lot::Mutex;
use rvoip_core::adapter::ConnectionAdapter;
use rvoip_core::commands::InboundAction;
use rvoip_core::config::Config;
use rvoip_core::events::Event;
use rvoip_core::ids::{ConnectionId, ParticipantId, SessionId};
use rvoip_core::orchestrator::Orchestrator;
use rvoip_webrtc::media::fixtures::send_fixture_media_burst;
use rvoip_webrtc::peer::RvoipPeerConnection;
use rvoip_webrtc::{IceServerConfig, WebRtcAdapter, WebRtcConfig, WebRtcServerBuilder};
use tokio::net::TcpListener;
use webrtc::data_channel::{DataChannel, DataChannelEvent};

const CHAT_ECHO_PREFIX: &str = "echo:";

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    tracing_subscriber::fmt::init();
    let _ = rustls::crypto::ring::default_provider().install_default();

    let mut config = WebRtcConfig::loopback();
    config.cors_origins = vec!["*".into()];
    config.trickle_ice = true;
    // RFC 9725 §4.6 — when the server has STUN/TURN configured, the WHIP
    // response advertises them via `Link: <url>; rel="ice-server"`. Set a
    // real STUN URL so the Playwright suite can assert on the header.
    config.ice_servers = vec![IceServerConfig::stun("stun:stun.l.google.com:19302")];
    // Outbound (WHEP) offers should include `m=video` so the receiver-side
    // RFC 9725 spec can verify bidirectional A+V. Without this the offerer
    // path auto-attaches only audio.
    config.originate_include_video = true;

    let whip_bind = std::env::var("WHIP_BIND").unwrap_or_else(|_| "127.0.0.1:0".into());
    let ws_bind = std::env::var("WS_BIND").unwrap_or_else(|_| "127.0.0.1:0".into());
    let static_bind = std::env::var("STATIC_BIND").unwrap_or_else(|_| "127.0.0.1:0".into());

    let server = WebRtcServerBuilder::new(config)
        .with_whip(&whip_bind)
        .with_ws(&ws_bind)
        .build()
        .await?;
    let whip_addr = server.whip_addr().expect("whip listener");
    let ws_addr = server.ws_addr().expect("ws listener");
    let adapter = server.adapter();

    let static_addr = spawn_static_server(&static_bind).await?;

    let orchestrator = Arc::new(Orchestrator::new(Config::default()));
    orchestrator.register(adapter.clone() as Arc<dyn ConnectionAdapter>)?;

    println!(
        "[webrtc_browser_demo] READY whip=http://{whip_addr} ws=ws://{ws_addr} static=http://{static_addr}"
    );

    // Inbound peers (WHIP, WS-offer) need an explicit Accept routed through
    // the orchestrator before the server's SDP answer is published back to
    // the signaler. WHEP-originated outbound peers, in contrast, never fire
    // an inbound event — `adapter.originate` runs inside the WHEP POST
    // handler. The route-scanner below picks those up.
    let inbound_adapter = Arc::clone(&adapter);
    let inbound_orchestrator = Arc::clone(&orchestrator);
    tokio::spawn(async move {
        let mut events = inbound_orchestrator.subscribe_events();
        while let Ok(event) = events.recv().await {
            if let Event::ConnectionInbound { connection_id, .. } = event {
                if inbound_adapter.routes().contains_key(&connection_id) {
                    if let Err(e) = inbound_orchestrator
                        .route_inbound_connection(
                            connection_id,
                            InboundAction::Accept {
                                session_id: SessionId::new(),
                                participant_id: ParticipantId::new(),
                            },
                        )
                        .await
                    {
                        tracing::warn!(error = %e, "route_inbound_connection failed");
                    }
                }
            }
        }
    });

    // Scan adapter.routes() so we attach the same demo handler (fixture
    // media burst + DC echo) to every peer regardless of which signaling
    // surface allocated it (WHIP inbound, WS inbound, WHEP outbound).
    let seen: Arc<Mutex<HashSet<ConnectionId>>> = Arc::new(Mutex::new(HashSet::new()));
    loop {
        let new_ids: Vec<ConnectionId> = {
            let routes = adapter.routes();
            let mut seen = seen.lock();
            routes
                .iter()
                .map(|e| e.key().clone())
                .filter(|id| seen.insert(id.clone()))
                .collect()
        };
        for id in new_ids {
            let adapter_spawn = Arc::clone(&adapter);
            tokio::spawn(async move {
                handle_browser_demo_connection(adapter_spawn, id).await;
            });
        }
        tokio::time::sleep(Duration::from_millis(20)).await;
    }
}

async fn handle_browser_demo_connection(adapter: Arc<WebRtcAdapter>, connection_id: ConnectionId) {
    let peer = {
        let Some(route) = adapter.routes().get(&connection_id) else {
            return;
        };
        let peer = route.peer.clone();
        drop(route);
        peer
    };

    if peer.wait_connected(Duration::from_secs(15)).await.is_err() {
        return;
    }

    // Continuously emit fixture audio+video so the browser observes
    // monotonically-growing `bytesReceived` via `getStats()` for the full
    // duration of the Playwright check, not just the 500ms one-shot burst.
    // Bail when the route disappears so we don't spin forever after the
    // peer hangs up.
    let peer_media = Arc::clone(&peer);
    let adapter_media = Arc::clone(&adapter);
    let conn_media = connection_id.clone();
    tokio::spawn(async move {
        while adapter_media.routes().contains_key(&conn_media) {
            send_fixture_media_burst(&peer_media, peer_media.local_video_track().is_some()).await;
        }
    });

    // Echo every data channel the browser opens. The Playwright DC spec
    // opens three (reliable / unreliable-retransmits / partial-lifetime);
    // we don't care about the labels here — we echo whatever arrives.
    // Poll on a short timeout so WHIP-only flows (no DC) exit promptly
    // when the route is reaped rather than blocking the task for ~60s.
    while adapter.routes().contains_key(&connection_id) {
        if let Some(dc) = peer.wait_data_channel(Duration::from_millis(500)).await {
            tokio::spawn(echo_data_channel(dc));
        }
    }
}

async fn echo_data_channel(dc: Arc<dyn DataChannel>) {
    loop {
        let Some(event) =
            RvoipPeerConnection::poll_data_channel(&dc, Duration::from_millis(200)).await
        else {
            continue;
        };
        match event {
            DataChannelEvent::OnMessage(msg) if msg.is_string => {
                let text = String::from_utf8_lossy(&msg.data);
                if text == "ping" {
                    let _ = dc.send_text("pong").await;
                } else {
                    let _ = dc.send_text(&format!("{CHAT_ECHO_PREFIX}{text}")).await;
                }
            }
            DataChannelEvent::OnClose => return,
            _ => {}
        }
    }
}

async fn spawn_static_server(
    bind: &str,
) -> Result<std::net::SocketAddr, Box<dyn std::error::Error>> {
    let root = PathBuf::from(env!("CARGO_MANIFEST_DIR")).join("static");
    let app = Router::new()
        .route("/", get(|| async { axum::response::Redirect::temporary("/whip-publish.html") }))
        .route(
            "/:file",
            get(move |Path(file): Path<String>| {
                let root = root.clone();
                async move { serve_file(&root, &file).await }
            }),
        );
    let listener = TcpListener::bind(bind).await?;
    let addr = listener.local_addr()?;
    tokio::spawn(async move {
        let _ = axum::serve(listener, app).await;
    });
    Ok(addr)
}

async fn serve_file(root: &PathBuf, file: &str) -> impl IntoResponse {
    let safe = file
        .chars()
        .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '-' | '_'));
    if !safe {
        return (StatusCode::BAD_REQUEST, "bad filename").into_response();
    }
    let path = root.join(file);
    match tokio::fs::read(&path).await {
        Ok(bytes) => {
            let mut headers = HeaderMap::new();
            let ct = if file.ends_with(".html") {
                "text/html; charset=utf-8"
            } else if file.ends_with(".js") {
                "application/javascript"
            } else if file.ends_with(".css") {
                "text/css; charset=utf-8"
            } else {
                "application/octet-stream"
            };
            headers.insert("content-type", HeaderValue::from_static(ct));
            (StatusCode::OK, headers, bytes).into_response()
        }
        Err(_) => (StatusCode::NOT_FOUND, "not found").into_response(),
    }
}
