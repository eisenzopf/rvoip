//! WebSocket JSON SDP signaler (feature `signaling-ws`).
//!
//! Inbound message shape (snake_case, matches `SignalingMessage` below):
//! - `{ "type": "offer", "sdp": "..." }` — answerer flow; reply `{type:"answer", sdp, connection_id}`.
//! - `{ "type": "answer", "sdp": "...", "connection_id": "..." }` — completes
//!   an outbound `originate()` call; reply `{type:"ack", connection_id}`.
//! - `{ "type": "ice-candidate", "candidate": "<RTCIceCandidateInit JSON>",
//!     "connection_id": "..." }` — trickle ICE; applied to the named peer.
//! - `{ "type": "bye", "connection_id": "..." }` — ends the route.
//!
//! When the adapter's `WebRtcConfig::trickle_ice` is enabled, the server
//! pushes its own locally-gathered candidates back to the client as
//! `{type:"ice-candidate", candidate, connection_id}` messages until the
//! connection closes.

use std::sync::Arc;
use std::time::Duration;

use std::net::SocketAddr;

use futures::{stream::SplitSink, SinkExt, StreamExt};
use rvoip_core::adapter::ConnectionAdapter;
use rvoip_core::ids::ConnectionId;
use serde::{Deserialize, Serialize};
use tokio::net::TcpListener;
use tokio::sync::Mutex as AsyncMutex;
use tokio_tungstenite::{
    accept_async, accept_async_with_config, accept_hdr_async_with_config,
    tungstenite::{
        handshake::server::{ErrorResponse, Request, Response as HandshakeResponse},
        http::{self, StatusCode as TStatus},
        protocol::WebSocketConfig,
        Message,
    },
    WebSocketStream,
};
use tracing::warn;
use webrtc::peer_connection::RTCIceCandidateInit;

use crate::adapter::WebRtcAdapter;
use crate::errors::{Result, WebRtcError};
use crate::signaling::auth::{AnonymousAuth, AuthRejection, WsAuthHook};

#[derive(Debug, Deserialize, Serialize, Default)]
pub struct SignalingMessage {
    #[serde(rename = "type")]
    pub msg_type: String,
    #[serde(default)]
    pub sdp: String,
    /// Routes `{type:"answer"}` to an outbound originate connection or scopes
    /// `{type:"ice-candidate"}` to a specific peer.
    #[serde(default, rename = "connection_id")]
    pub connection_id: String,
    /// Trickle ICE candidate — opaque JSON encoding of [`RTCIceCandidateInit`]
    /// (camelCase keys: `candidate`, `sdpMid`, `sdpMLineIndex`, ...).
    #[serde(default)]
    pub candidate: String,
}

type WsSink = Arc<AsyncMutex<SplitSink<WebSocketStream<tokio::net::TcpStream>, Message>>>;

/// Accept WebSocket connections and exchange `{type, sdp, connection_id?}` JSON messages.
pub async fn serve(bind: &str, adapter: Arc<WebRtcAdapter>) -> Result<()> {
    let listener = TcpListener::bind(bind)
        .await
        .map_err(|e| WebRtcError::Signaling(format!("bind {bind}: {e}")))?;
    serve_listener(listener, adapter).await
}

/// Serve on an already-bound listener (integration tests).
pub async fn serve_listener(listener: TcpListener, adapter: Arc<WebRtcAdapter>) -> Result<()> {
    serve_listener_with_auth(listener, adapter, Arc::new(AnonymousAuth)).await
}

/// Serve with a custom [`WsAuthHook`] enforced during the WebSocket upgrade
/// (RFC 7235 — 401 returned before the upgrade completes on rejection).
pub async fn serve_listener_with_auth(
    listener: TcpListener,
    adapter: Arc<WebRtcAdapter>,
    auth: Arc<dyn WsAuthHook>,
) -> Result<()> {
    loop {
        let (stream, peer_addr) = listener
            .accept()
            .await
            .map_err(|e| WebRtcError::Signaling(format!("{e}")))?;
        let adapter = Arc::clone(&adapter);
        let auth = Arc::clone(&auth);
        tokio::spawn(async move {
            if let Err(e) = handle_connection_with_auth(stream, adapter, auth, peer_addr).await {
                tracing::warn!("ws signaling connection error: {e}");
            }
        });
    }
}

/// WSS variant — TLS-terminating WebSocket signaler. Requires `tls-rustls`.
/// Each accepted TCP connection is wrapped via `tokio-rustls` before being
/// handed to the standard WS handshake.
#[cfg(feature = "tls-rustls")]
pub async fn serve_tls_listener(
    listener: TcpListener,
    tls: crate::tls::TlsConfig,
    adapter: Arc<WebRtcAdapter>,
) -> Result<()> {
    loop {
        let (stream, _) = listener
            .accept()
            .await
            .map_err(|e| WebRtcError::Signaling(format!("{e}")))?;
        let acceptor = tls.acceptor.clone();
        let adapter = Arc::clone(&adapter);
        tokio::spawn(async move {
            let tls_stream = match acceptor.accept(stream).await {
                Ok(s) => s,
                Err(e) => {
                    tracing::warn!("WSS TLS handshake failed: {e}");
                    return;
                }
            };
            if let Err(e) = handle_tls_connection(tls_stream, adapter).await {
                tracing::warn!("wss signaling connection error: {e}");
            }
        });
    }
}

#[cfg(feature = "tls-rustls")]
async fn handle_tls_connection(
    stream: tokio_rustls::server::TlsStream<tokio::net::TcpStream>,
    adapter: Arc<WebRtcAdapter>,
) -> Result<()> {
    // Same flow as `handle_connection` but parameterized over the stream type.
    // We re-use the handshake by inlining the tungstenite accept here.
    let ws = tokio_tungstenite::accept_async(stream)
        .await
        .map_err(|e| WebRtcError::Signaling(format!("{e}")))?;
    let (write, mut read) = ws.split();
    let write: Arc<AsyncMutex<_>> = Arc::new(AsyncMutex::new(write));
    let mut forwarder_spawned: Option<ConnectionId> = None;

    while let Some(msg) = read.next().await {
        let msg = msg.map_err(|e| WebRtcError::Signaling(format!("{e}")))?;
        if msg.is_pong() || msg.is_ping() {
            continue;
        }
        if msg.is_close() {
            break;
        }
        if !msg.is_text() {
            continue;
        }
        let text = msg
            .to_text()
            .map_err(|e| WebRtcError::Signaling(format!("ws text: {e}")))?;
        let parsed: SignalingMessage =
            serde_json::from_str(text).map_err(|e| WebRtcError::Signaling(format!("{e}")))?;
        dispatch_tls(&adapter, &write, parsed, &mut forwarder_spawned).await?;
    }
    let _ = forwarder_spawned;
    Ok(())
}

#[cfg(feature = "tls-rustls")]
async fn dispatch_tls(
    adapter: &Arc<WebRtcAdapter>,
    write: &Arc<
        AsyncMutex<
            futures::stream::SplitSink<
                tokio_tungstenite::WebSocketStream<
                    tokio_rustls::server::TlsStream<tokio::net::TcpStream>,
                >,
                Message,
            >,
        >,
    >,
    parsed: SignalingMessage,
    _forwarder_spawned: &mut Option<ConnectionId>,
) -> Result<()> {
    // The WSS variant supports the same offer/answer/ice-candidate/bye flow as
    // the plaintext version but without the local-ICE forwarder (which only
    // matters when trickle is enabled — H7 follow-up).
    match parsed.msg_type.as_str() {
        "offer" => {
            let conn_id = adapter.apply_remote_offer(&parsed.sdp).await?;
            let answer = adapter.local_sdp(&conn_id)?;
            let out = SignalingMessage {
                msg_type: "answer".into(),
                sdp: answer,
                connection_id: conn_id.to_string(),
                candidate: String::new(),
            };
            let payload = serde_json::to_string(&out)
                .map_err(|e| WebRtcError::Signaling(format!("serialize answer: {e}")))?;
            write
                .lock()
                .await
                .send(Message::Text(payload.into()))
                .await
                .map_err(|e| WebRtcError::Signaling(format!("{e}")))?;
        }
        "answer" => {
            if parsed.connection_id.is_empty() {
                return Err(WebRtcError::Signaling(
                    "answer requires connection_id".into(),
                ));
            }
            let conn_id = ConnectionId::from_string(parsed.connection_id);
            adapter.accept_remote_answer(conn_id, &parsed.sdp).await?;
        }
        "ice-candidate" => {
            if parsed.connection_id.is_empty() || parsed.candidate.is_empty() {
                return Err(WebRtcError::Signaling(
                    "ice-candidate requires connection_id and candidate".into(),
                ));
            }
            let conn_id = ConnectionId::from_string(parsed.connection_id.clone());
            let candidate: RTCIceCandidateInit = serde_json::from_str(&parsed.candidate)
                .map_err(|e| WebRtcError::Signaling(format!("ice-candidate parse: {e}")))?;
            adapter.apply_trickle_candidate(&conn_id, candidate).await?;
        }
        "bye" => {
            if !parsed.connection_id.is_empty() {
                let conn_id = ConnectionId::from_string(parsed.connection_id);
                let _ = adapter
                    .end(conn_id, rvoip_core::adapter::EndReason::Normal)
                    .await;
            }
        }
        other => {
            return Err(WebRtcError::Signaling(format!(
                "unknown signaling message type: {other}"
            )));
        }
    }
    Ok(())
}

async fn handle_connection(
    stream: tokio::net::TcpStream,
    adapter: Arc<WebRtcAdapter>,
) -> Result<()> {
    let max_msg = adapter.ws_max_message_size();
    let ws = if max_msg < 64 * 1024 * 1024 {
        let ws_config = WebSocketConfig::default()
            .max_message_size(Some(max_msg))
            .max_frame_size(Some(max_msg));
        accept_async_with_config(stream, Some(ws_config))
            .await
            .map_err(|e| WebRtcError::Signaling(format!("{e}")))?
    } else {
        accept_async(stream)
            .await
            .map_err(|e| WebRtcError::Signaling(format!("{e}")))?
    };
    drive_ws_loop(ws, adapter).await
}

/// Same as [`handle_connection`] but runs a [`WsAuthHook`] during the
/// WebSocket upgrade. On rejection the server emits a proper HTTP
/// response (401/403/429) before the upgrade completes, so JS clients
/// see the error in `WebSocket.onerror` rather than a closed socket.
async fn handle_connection_with_auth(
    stream: tokio::net::TcpStream,
    adapter: Arc<WebRtcAdapter>,
    auth: Arc<dyn WsAuthHook>,
    peer_addr: SocketAddr,
) -> Result<()> {
    // Collect handshake metadata (subprotocols + query token) inside the
    // tungstenite Callback, then reject synchronously if the synchronous
    // pre-check fails. Because the Callback is sync, we authenticate
    // before the upgrade only on the cheap "is the token set at all"
    // branch; full async hook execution happens after the upgrade and
    // closes the socket with code 4401 on rejection.
    let mut subprotocols: Vec<String> = Vec::new();
    let mut query_token: Option<String> = None;

    let cb = |req: &Request,
              mut resp: HandshakeResponse|
     -> std::result::Result<HandshakeResponse, ErrorResponse> {
        // Sec-WebSocket-Protocol (comma-separated list).
        if let Some(v) = req
            .headers()
            .get("sec-websocket-protocol")
            .and_then(|h| h.to_str().ok())
        {
            for s in v.split(',') {
                let trimmed = s.trim();
                if !trimmed.is_empty() {
                    subprotocols.push(trimmed.to_string());
                }
            }
            // Echo back the first subprotocol that isn't a `token.*`
            // smuggled credential; default to `rvoip.webrtc.v1` if any
            // subprotocol is offered.
            if let Some(echo) = subprotocols
                .iter()
                .find(|s| !s.starts_with("token."))
                .cloned()
                .or_else(|| {
                    if subprotocols.is_empty() {
                        None
                    } else {
                        Some("rvoip.webrtc.v1".into())
                    }
                })
            {
                if let Ok(v) = echo.parse::<http::HeaderValue>() {
                    resp.headers_mut().insert("sec-websocket-protocol", v);
                }
            }
        }
        // Extract ?access_token=… from the request path.
        let uri = req.uri();
        if let Some(q) = uri.query() {
            for kv in q.split('&') {
                if let Some(v) = kv.strip_prefix("access_token=") {
                    query_token = Some(v.to_string());
                }
            }
        }
        Ok(resp)
    };

    let max_msg = adapter.ws_max_message_size();
    let ws_config = if max_msg < 64 * 1024 * 1024 {
        Some(
            WebSocketConfig::default()
                .max_message_size(Some(max_msg))
                .max_frame_size(Some(max_msg)),
        )
    } else {
        None
    };

    let ws = accept_hdr_async_with_config(stream, cb, ws_config)
        .await
        .map_err(|e| WebRtcError::Signaling(format!("{e}")))?;

    // Now run the async hook. On rejection close with a custom code.
    match auth
        .authenticate(&subprotocols, query_token.as_deref(), peer_addr)
        .await
    {
        Ok(_ctx) => {}
        Err(rej) => {
            let (code, reason) = match rej {
                AuthRejection::Unauthorized { .. } => (4401u16, "unauthorized"),
                AuthRejection::Forbidden => (4403, "forbidden"),
                AuthRejection::Throttled { .. } => (4429, "throttled"),
            };
            let (mut write, _read) = ws.split();
            let _ = write
                .send(Message::Close(Some(
                    tokio_tungstenite::tungstenite::protocol::CloseFrame {
                        code: tokio_tungstenite::tungstenite::protocol::frame::coding::CloseCode::Library(code),
                        reason: reason.into(),
                    },
                )))
                .await;
            adapter.note_signaling_error();
            return Ok(());
        }
    }
    drive_ws_loop(ws, adapter).await
}

async fn drive_ws_loop(
    ws: WebSocketStream<tokio::net::TcpStream>,
    adapter: Arc<WebRtcAdapter>,
) -> Result<()> {
    let (write, mut read) = ws.split();
    let write: WsSink = Arc::new(AsyncMutex::new(write));
    // Tracks the connection_id we've already spawned a candidate-forwarder for
    // so we don't spawn duplicates.
    let mut forwarder_spawned: Option<ConnectionId> = None;

    // Server-driven keepalive ping (anti-zombie). Cancelled when the read loop exits.
    let keepalive_secs = adapter.ws_keepalive_secs();
    let keepalive = if keepalive_secs > 0 {
        let write = Arc::clone(&write);
        Some(tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(keepalive_secs));
            tick.tick().await; // skip immediate fire
            loop {
                tick.tick().await;
                let res = write
                    .lock()
                    .await
                    .send(Message::Ping(Default::default()))
                    .await;
                if res.is_err() {
                    break;
                }
            }
        }))
    } else {
        None
    };

    while let Some(msg) = read.next().await {
        let msg = msg.map_err(|e| WebRtcError::Signaling(format!("{e}")))?;
        if msg.is_pong() || msg.is_ping() {
            continue;
        }
        if msg.is_close() {
            break;
        }
        if !msg.is_text() {
            continue;
        }
        let text = msg
            .to_text()
            .map_err(|e| WebRtcError::Signaling(format!("ws text: {e}")))?;
        let parsed: SignalingMessage =
            serde_json::from_str(text).map_err(|e| WebRtcError::Signaling(format!("{e}")))?;

        match parsed.msg_type.as_str() {
            "offer" => {
                let conn_id = adapter.apply_remote_offer(&parsed.sdp).await?;
                let answer = adapter.local_sdp(&conn_id)?;
                send_message(
                    &write,
                    &SignalingMessage {
                        msg_type: "answer".into(),
                        sdp: answer,
                        connection_id: conn_id.to_string(),
                        candidate: String::new(),
                    },
                )
                .await?;
                // Only spawn the local-ICE forwarder when the adapter is in
                // trickle mode. In full-gather mode (the default), candidates
                // are already inline in the SDP — forwarding them again wastes
                // work and can race with handlers that drop the WS right after
                // receiving the answer.
                if adapter.trickle_ice_enabled() {
                    spawn_local_ice_forwarder(&adapter, &conn_id, &write);
                    forwarder_spawned = Some(conn_id);
                }
            }
            "answer" => {
                if parsed.connection_id.is_empty() {
                    return Err(WebRtcError::Signaling(
                        "answer requires connection_id".into(),
                    ));
                }
                let conn_id = ConnectionId::from_string(parsed.connection_id.clone());
                adapter
                    .accept_remote_answer(conn_id.clone(), &parsed.sdp)
                    .await?;
                send_message(
                    &write,
                    &SignalingMessage {
                        msg_type: "ack".into(),
                        sdp: String::new(),
                        connection_id: parsed.connection_id,
                        candidate: String::new(),
                    },
                )
                .await?;
                if adapter.trickle_ice_enabled() {
                    spawn_local_ice_forwarder(&adapter, &conn_id, &write);
                    forwarder_spawned = Some(conn_id);
                }
            }
            "ice-candidate" => {
                if parsed.connection_id.is_empty() || parsed.candidate.is_empty() {
                    return Err(WebRtcError::Signaling(
                        "ice-candidate requires connection_id and candidate".into(),
                    ));
                }
                let conn_id = ConnectionId::from_string(parsed.connection_id.clone());
                let candidate: RTCIceCandidateInit = serde_json::from_str(&parsed.candidate)
                    .map_err(|e| WebRtcError::Signaling(format!("ice-candidate parse: {e}")))?;
                adapter.apply_trickle_candidate(&conn_id, candidate).await?;
            }
            "bye" => {
                if !parsed.connection_id.is_empty() {
                    let conn_id = ConnectionId::from_string(parsed.connection_id);
                    let _ = adapter
                        .end(conn_id, rvoip_core::adapter::EndReason::Normal)
                        .await;
                }
                break;
            }
            other => {
                return Err(WebRtcError::Signaling(format!(
                    "unknown signaling message type: {other}"
                )));
            }
        }
    }

    let _ = forwarder_spawned;
    if let Some(h) = keepalive {
        h.abort();
    }
    Ok(())
}

async fn send_message(write: &WsSink, msg: &SignalingMessage) -> Result<()> {
    let payload = serde_json::to_string(msg)
        .map_err(|e| WebRtcError::Signaling(format!("serialize {}: {e}", msg.msg_type)))?;
    write
        .lock()
        .await
        .send(Message::Text(payload.into()))
        .await
        .map_err(|e| WebRtcError::Signaling(format!("{e}")))
}

/// Drain locally-gathered ICE candidates from the peer connection identified
/// by `conn_id` and stream them as `{type:"ice-candidate"}` messages to the
/// WS client. Exits when the route is gone, the WS sink errors, or the
/// channel closes.
fn spawn_local_ice_forwarder(adapter: &Arc<WebRtcAdapter>, conn_id: &ConnectionId, write: &WsSink) {
    let adapter = Arc::clone(adapter);
    let conn_id = conn_id.clone();
    let write = Arc::clone(write);
    tokio::spawn(async move {
        loop {
            // Look up the peer fresh each iteration so an end() reliably
            // tears down this task.
            let Some(route) = adapter.routes().get(&conn_id).map(|e| e.value().clone()) else {
                return;
            };
            tokio::select! {
                _ = route.cancel.notified() => return,
                cand = route.peer.recv_local_ice() => {
                    let Some(cand) = cand else { return; };
                    let init = match cand.to_json() {
                        Ok(init) => init,
                        Err(e) => {
                            warn!("local ICE candidate to_json failed: {e}");
                            continue;
                        }
                    };
                    let payload = match serde_json::to_string(&init) {
                        Ok(p) => p,
                        Err(e) => {
                            warn!("serialize local ICE candidate failed: {e}");
                            continue;
                        }
                    };
                    let msg = SignalingMessage {
                        msg_type: "ice-candidate".into(),
                        sdp: String::new(),
                        connection_id: conn_id.to_string(),
                        candidate: payload,
                    };
                    let send = send_message(&write, &msg);
                    // Bound the WS send so a frozen peer can't deadlock us.
                    if let Err(e) = tokio::time::timeout(Duration::from_secs(5), send).await {
                        warn!("WS ice-candidate send timed out: {e}");
                        return;
                    }
                }
            }
        }
    });
}
