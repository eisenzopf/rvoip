//! WebSocket JSON SDP signaler (feature `signaling-ws`).
//!
//! Authentication is completed before the HTTP 101 response for both WS and
//! WSS. Once upgraded, every signaling mutation is authorized against the
//! adapter-owned route identity shared with WHIP/WHEP.

use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use futures::{stream::SplitSink, SinkExt, StreamExt};
use rvoip_core::adapter::InboundRoutingHint;
use rvoip_core::ids::ConnectionId;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::TcpListener;
use tokio::sync::Mutex as AsyncMutex;
use tokio_tungstenite::{
    accept_hdr_async_with_config,
    tungstenite::{
        handshake::server::{ErrorResponse, Request, Response as HandshakeResponse},
        http,
        protocol::WebSocketConfig,
        Message,
    },
    WebSocketStream,
};
use tracing::warn;
use webrtc::peer_connection::RTCIceCandidateInit;

use crate::adapter::{RouteAuthorization, WebRtcAdapter};
use crate::errors::{Result, WebRtcError};
use crate::signaling::auth::{AnonymousAuth, AuthContext, AuthRejection, WsAuthHook};

const MAX_HANDSHAKE_BYTES: usize = 16 * 1024;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Debug, Deserialize, Serialize, Default)]
pub struct SignalingMessage {
    #[serde(rename = "type")]
    pub msg_type: String,
    #[serde(default)]
    pub sdp: String,
    #[serde(default, rename = "connection_id")]
    pub connection_id: String,
    #[serde(default)]
    pub candidate: String,
}

/// Stream wrapper which replays the HTTP handshake bytes inspected during
/// asynchronous authentication before delegating to the underlying socket.
struct PrefixedStream<S> {
    prefix: Vec<u8>,
    offset: usize,
    inner: S,
}

impl<S> PrefixedStream<S> {
    fn new(prefix: Vec<u8>, inner: S) -> Self {
        Self {
            prefix,
            offset: 0,
            inner,
        }
    }
}

impl<S: AsyncRead + Unpin> AsyncRead for PrefixedStream<S> {
    fn poll_read(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        if self.offset < self.prefix.len() {
            let remaining = &self.prefix[self.offset..];
            let count = remaining.len().min(buf.remaining());
            buf.put_slice(&remaining[..count]);
            self.offset += count;
            return Poll::Ready(Ok(()));
        }
        Pin::new(&mut self.inner).poll_read(cx, buf)
    }
}

impl<S: AsyncWrite + Unpin> AsyncWrite for PrefixedStream<S> {
    fn poll_write(
        mut self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buf: &[u8],
    ) -> Poll<std::io::Result<usize>> {
        Pin::new(&mut self.inner).poll_write(cx, buf)
    }

    fn poll_flush(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_flush(cx)
    }

    fn poll_shutdown(mut self: Pin<&mut Self>, cx: &mut Context<'_>) -> Poll<std::io::Result<()>> {
        Pin::new(&mut self.inner).poll_shutdown(cx)
    }
}

type WsSink<S> = Arc<AsyncMutex<SplitSink<WebSocketStream<S>, Message>>>;

#[derive(Debug)]
struct HandshakeMetadata {
    subprotocols: Vec<String>,
    query_token: Option<String>,
}

/// Accept WebSocket connections and exchange JSON signaling messages.
pub async fn serve(bind: &str, adapter: Arc<WebRtcAdapter>) -> Result<()> {
    let listener = TcpListener::bind(bind)
        .await
        .map_err(|e| WebRtcError::Signaling(format!("bind {bind}: {e}")))?;
    serve_listener(listener, adapter).await
}

pub async fn serve_listener(listener: TcpListener, adapter: Arc<WebRtcAdapter>) -> Result<()> {
    serve_listener_with_auth(listener, adapter, Arc::new(AnonymousAuth)).await
}

/// Serve with a custom auth hook. The hook is awaited before the WebSocket
/// handshake is allowed to emit HTTP 101.
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
            if let Err(error) = handle_authenticated_stream(stream, adapter, auth, peer_addr).await
            {
                warn!("ws signaling connection error: {error}");
            }
        });
    }
}

#[cfg(feature = "tls-rustls")]
pub async fn serve_tls_listener(
    listener: TcpListener,
    tls: crate::tls::TlsConfig,
    adapter: Arc<WebRtcAdapter>,
) -> Result<()> {
    serve_tls_listener_with_auth(listener, tls, adapter, Arc::new(AnonymousAuth)).await
}

#[cfg(feature = "tls-rustls")]
pub async fn serve_tls_listener_with_auth(
    listener: TcpListener,
    tls: crate::tls::TlsConfig,
    adapter: Arc<WebRtcAdapter>,
    auth: Arc<dyn WsAuthHook>,
) -> Result<()> {
    loop {
        let (stream, peer_addr) = listener
            .accept()
            .await
            .map_err(|e| WebRtcError::Signaling(format!("{e}")))?;
        let acceptor = tls.acceptor.clone();
        let adapter = Arc::clone(&adapter);
        let auth = Arc::clone(&auth);
        tokio::spawn(async move {
            let stream = match acceptor.accept(stream).await {
                Ok(stream) => stream,
                Err(error) => {
                    warn!("WSS TLS handshake failed: {error}");
                    return;
                }
            };
            if let Err(error) = handle_authenticated_stream(stream, adapter, auth, peer_addr).await
            {
                warn!("wss signaling connection error: {error}");
            }
        });
    }
}

async fn handle_authenticated_stream<S>(
    stream: S,
    adapter: Arc<WebRtcAdapter>,
    auth: Arc<dyn WsAuthHook>,
    peer_addr: SocketAddr,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let Some((ws, auth_context)) =
        upgrade_with_auth(stream, &adapter, auth.as_ref(), peer_addr).await?
    else {
        return Ok(());
    };
    drive_ws_loop(ws, adapter, auth_context).await
}

async fn upgrade_with_auth<S>(
    mut stream: S,
    adapter: &Arc<WebRtcAdapter>,
    auth: &dyn WsAuthHook,
    peer_addr: SocketAddr,
) -> Result<Option<(WebSocketStream<PrefixedStream<S>>, AuthContext)>>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let prefix = tokio::time::timeout(HANDSHAKE_TIMEOUT, read_http_handshake(&mut stream))
        .await
        .map_err(|_| WebRtcError::Signaling("websocket handshake timed out".into()))??;
    let metadata = parse_handshake_metadata(&prefix)?;

    let auth_context = match auth
        .authenticate(
            &metadata.subprotocols,
            metadata.query_token.as_deref(),
            peer_addr,
        )
        .await
    {
        Ok(context) => context,
        Err(rejection) => {
            adapter.note_signaling_error();
            write_auth_rejection(&mut stream, rejection).await?;
            return Ok(None);
        }
    };

    let response_protocol = select_response_protocol(&metadata.subprotocols, auth);
    let callback = move |_request: &Request,
                         mut response: HandshakeResponse|
          -> std::result::Result<HandshakeResponse, ErrorResponse> {
        if let Some(protocol) = response_protocol.as_deref() {
            if let Ok(value) = protocol.parse::<http::HeaderValue>() {
                response
                    .headers_mut()
                    .insert("sec-websocket-protocol", value);
            }
        }
        Ok(response)
    };
    let max_message_size = adapter.ws_max_message_size();
    let config = (max_message_size < 64 * 1024 * 1024).then(|| {
        WebSocketConfig::default()
            .max_message_size(Some(max_message_size))
            .max_frame_size(Some(max_message_size))
    });
    let stream = PrefixedStream::new(prefix, stream);
    let ws = accept_hdr_async_with_config(stream, callback, config)
        .await
        .map_err(|error| WebRtcError::Signaling(format!("{error}")))?;
    Ok(Some((ws, auth_context)))
}

async fn read_http_handshake<S: AsyncRead + Unpin>(stream: &mut S) -> Result<Vec<u8>> {
    let mut bytes = Vec::with_capacity(1024);
    let mut chunk = [0u8; 1024];
    loop {
        if bytes.windows(4).any(|window| window == b"\r\n\r\n") {
            return Ok(bytes);
        }
        if bytes.len() >= MAX_HANDSHAKE_BYTES {
            return Err(WebRtcError::Signaling(
                "websocket handshake headers exceed 16 KiB".into(),
            ));
        }
        let read_limit = chunk.len().min(MAX_HANDSHAKE_BYTES - bytes.len());
        let read = stream
            .read(&mut chunk[..read_limit])
            .await
            .map_err(|error| WebRtcError::Signaling(format!("read handshake: {error}")))?;
        if read == 0 {
            return Err(WebRtcError::Signaling(
                "connection closed during websocket handshake".into(),
            ));
        }
        bytes.extend_from_slice(&chunk[..read]);
    }
}

fn parse_handshake_metadata(bytes: &[u8]) -> Result<HandshakeMetadata> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| WebRtcError::Signaling("websocket handshake is not valid UTF-8".into()))?;
    let headers_end = text
        .find("\r\n\r\n")
        .ok_or_else(|| WebRtcError::Signaling("incomplete websocket handshake".into()))?;
    let mut lines = text[..headers_end].split("\r\n");
    let request_line = lines
        .next()
        .ok_or_else(|| WebRtcError::Signaling("missing websocket request line".into()))?;
    let mut request_parts = request_line.split_whitespace();
    let method = request_parts.next().unwrap_or_default();
    let target = request_parts.next().unwrap_or_default();
    if method != "GET" || target.is_empty() {
        return Err(WebRtcError::Signaling(
            "invalid websocket request line".into(),
        ));
    }

    let mut subprotocols = Vec::new();
    for line in lines {
        let Some((name, value)) = line.split_once(':') else {
            continue;
        };
        if name.eq_ignore_ascii_case("sec-websocket-protocol") {
            subprotocols.extend(
                value
                    .split(',')
                    .map(str::trim)
                    .filter(|value| !value.is_empty())
                    .map(str::to_owned),
            );
        }
    }
    let query_token = target.split_once('?').and_then(|(_, query)| {
        query.split('&').find_map(|pair| {
            pair.strip_prefix("access_token=")
                .filter(|value| !value.is_empty())
                .map(str::to_owned)
        })
    });
    Ok(HandshakeMetadata {
        subprotocols,
        query_token,
    })
}

fn select_response_protocol(subprotocols: &[String], auth: &dyn WsAuthHook) -> Option<String> {
    subprotocols
        .iter()
        .find(|value| !auth.subprotocol_is_private(value))
        .cloned()
        .or_else(|| (!subprotocols.is_empty()).then(|| "rvoip.webrtc.v1".to_string()))
}

async fn write_auth_rejection<S: AsyncWrite + Unpin>(
    stream: &mut S,
    rejection: AuthRejection,
) -> Result<()> {
    let (status, reason, header) = match rejection {
        AuthRejection::Unauthorized { www_authenticate } => {
            let header = http::HeaderValue::from_str(&www_authenticate)
                .ok()
                .and_then(|value| value.to_str().ok().map(str::to_owned))
                .map(|value| format!("WWW-Authenticate: {value}\r\n"))
                .unwrap_or_default();
            (401, "Unauthorized", header)
        }
        AuthRejection::Forbidden => (403, "Forbidden", String::new()),
        AuthRejection::Throttled { retry_after_secs } => (
            429,
            "Too Many Requests",
            format!("Retry-After: {retry_after_secs}\r\n"),
        ),
    };
    let response = format!(
        "HTTP/1.1 {status} {reason}\r\nConnection: close\r\nCache-Control: no-store\r\n{header}Content-Length: 0\r\n\r\n"
    );
    stream
        .write_all(response.as_bytes())
        .await
        .map_err(|error| WebRtcError::Signaling(format!("write auth rejection: {error}")))?;
    stream
        .shutdown()
        .await
        .map_err(|error| WebRtcError::Signaling(format!("close rejected websocket: {error}")))
}

async fn drive_ws_loop<S>(
    ws: WebSocketStream<S>,
    adapter: Arc<WebRtcAdapter>,
    auth_context: AuthContext,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let routing_hint = auth_context.session_hint.clone();
    let authorization = auth_context.route_authorization();
    let (write, mut read) = ws.split();
    let write: WsSink<S> = Arc::new(AsyncMutex::new(write));
    let mut forwarders = Vec::new();

    let keepalive_secs = adapter.ws_keepalive_secs();
    let keepalive = if keepalive_secs > 0 {
        let write = Arc::clone(&write);
        Some(tokio::spawn(async move {
            let mut tick = tokio::time::interval(Duration::from_secs(keepalive_secs));
            tick.tick().await;
            loop {
                tick.tick().await;
                if write
                    .lock()
                    .await
                    .send(Message::Ping(Default::default()))
                    .await
                    .is_err()
                {
                    break;
                }
            }
        }))
    } else {
        None
    };

    let loop_result: Result<()> = async {
        while let Some(message) = read.next().await {
            let message = message.map_err(|error| WebRtcError::Signaling(format!("{error}")))?;
            if message.is_pong() || message.is_ping() {
                continue;
            }
            if message.is_close() {
                break;
            }
            if !message.is_text() {
                continue;
            }
            let parsed: SignalingMessage = serde_json::from_str(
                message
                    .to_text()
                    .map_err(|error| WebRtcError::Signaling(format!("ws text: {error}")))?,
            )
            .map_err(|error| WebRtcError::Signaling(format!("{error}")))?;
            let should_close = dispatch_message(
                &adapter,
                &write,
                parsed,
                &authorization,
                routing_hint.as_deref(),
                &mut forwarders,
            )
            .await?;
            if should_close {
                break;
            }
        }
        Ok(())
    }
    .await;

    if let Some(task) = keepalive {
        task.abort();
    }
    for (_, task) in forwarders {
        task.abort();
    }
    if loop_result.is_err() {
        adapter.note_signaling_error();
    }
    loop_result
}

async fn dispatch_message<S>(
    adapter: &Arc<WebRtcAdapter>,
    write: &WsSink<S>,
    parsed: SignalingMessage,
    authorization: &RouteAuthorization,
    routing_hint: Option<&str>,
    forwarders: &mut Vec<(ConnectionId, tokio::task::JoinHandle<()>)>,
) -> Result<bool>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    match parsed.msg_type.as_str() {
        "offer" => {
            let (conn_id, answer) = if parsed.connection_id.is_empty() {
                let routing_hint = routing_hint
                    .map(|value| InboundRoutingHint::new(value.to_owned()))
                    .transpose()
                    .map_err(|_| {
                        WebRtcError::Signaling(
                            "authenticated WebSocket session hint is invalid".into(),
                        )
                    })?;
                let conn_id = adapter
                    .apply_remote_offer_authorized_with_hint(
                        &parsed.sdp,
                        authorization.clone(),
                        routing_hint,
                    )
                    .await?;
                let answer = match adapter.local_sdp(&conn_id) {
                    Ok(answer) => answer,
                    Err(error) => {
                        let _ = adapter
                            .end_authorized(
                                conn_id,
                                rvoip_core::adapter::EndReason::Normal,
                                authorization,
                            )
                            .await;
                        return Err(error);
                    }
                };
                (conn_id, answer)
            } else {
                let conn_id = ConnectionId::from_string(parsed.connection_id);
                let answer = adapter
                    .apply_ice_restart_offer_authorized(conn_id.clone(), &parsed.sdp, authorization)
                    .await?;
                (conn_id, answer)
            };
            send_message(
                write,
                &SignalingMessage {
                    msg_type: "answer".into(),
                    sdp: answer,
                    connection_id: conn_id.to_string(),
                    candidate: String::new(),
                },
            )
            .await?;
            if adapter.trickle_ice_enabled() {
                ensure_local_ice_forwarder(adapter, &conn_id, write, forwarders);
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
                .accept_remote_answer_authorized(conn_id.clone(), &parsed.sdp, authorization)
                .await?;
            send_message(
                write,
                &SignalingMessage {
                    msg_type: "ack".into(),
                    sdp: String::new(),
                    connection_id: parsed.connection_id,
                    candidate: String::new(),
                },
            )
            .await?;
            if adapter.trickle_ice_enabled() {
                ensure_local_ice_forwarder(adapter, &conn_id, write, forwarders);
            }
        }
        "ice-candidate" => {
            if parsed.connection_id.is_empty() || parsed.candidate.is_empty() {
                return Err(WebRtcError::Signaling(
                    "ice-candidate requires connection_id and candidate".into(),
                ));
            }
            let conn_id = ConnectionId::from_string(parsed.connection_id);
            let candidate: RTCIceCandidateInit = serde_json::from_str(&parsed.candidate)
                .map_err(|error| WebRtcError::Signaling(format!("ice-candidate parse: {error}")))?;
            adapter
                .apply_trickle_candidate_authorized(&conn_id, candidate, authorization)
                .await?;
        }
        "bye" => {
            if !parsed.connection_id.is_empty() {
                adapter
                    .end_authorized(
                        ConnectionId::from_string(parsed.connection_id),
                        rvoip_core::adapter::EndReason::Normal,
                        authorization,
                    )
                    .await?;
            }
            return Ok(true);
        }
        other => {
            return Err(WebRtcError::Signaling(format!(
                "unknown signaling message type: {other}"
            )));
        }
    }
    Ok(false)
}

async fn send_message<S>(write: &WsSink<S>, message: &SignalingMessage) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin,
{
    let payload = serde_json::to_string(message).map_err(|error| {
        WebRtcError::Signaling(format!("serialize {}: {error}", message.msg_type))
    })?;
    write
        .lock()
        .await
        .send(Message::Text(payload.into()))
        .await
        .map_err(|error| WebRtcError::Signaling(format!("{error}")))
}

fn ensure_local_ice_forwarder<S>(
    adapter: &Arc<WebRtcAdapter>,
    conn_id: &ConnectionId,
    write: &WsSink<S>,
    forwarders: &mut Vec<(ConnectionId, tokio::task::JoinHandle<()>)>,
) where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    if let Some(index) = forwarders
        .iter()
        .position(|(existing, _)| existing == conn_id)
    {
        if !forwarders[index].1.is_finished() {
            return;
        }
        forwarders.swap_remove(index);
    }
    let adapter = Arc::clone(adapter);
    let conn_id = conn_id.clone();
    let write = Arc::clone(write);
    let task_conn_id = conn_id.clone();
    let task = tokio::spawn(async move {
        loop {
            let Some(route) = adapter
                .routes()
                .get(&conn_id)
                .map(|entry| entry.value().clone())
            else {
                return;
            };
            tokio::select! {
                _ = route.cancel.notified() => return,
                candidate = route.peer.recv_local_ice() => {
                    let Some(candidate) = candidate else { return; };
                    let init = match candidate.to_json() {
                        Ok(init) => init,
                        Err(error) => {
                            warn!("local ICE candidate to_json failed: {error}");
                            continue;
                        }
                    };
                    let payload = match serde_json::to_string(&init) {
                        Ok(payload) => payload,
                        Err(error) => {
                            warn!("serialize local ICE candidate failed: {error}");
                            continue;
                        }
                    };
                    let message = SignalingMessage {
                        msg_type: "ice-candidate".into(),
                        sdp: String::new(),
                        connection_id: conn_id.to_string(),
                        candidate: payload,
                    };
                    match tokio::time::timeout(
                        Duration::from_secs(5),
                        send_message(&write, &message),
                    )
                    .await
                    {
                        Ok(Ok(())) => {}
                        Ok(Err(error)) => {
                            warn!("WS ice-candidate send failed: {error}");
                            return;
                        }
                        Err(_) => {
                            warn!("WS ice-candidate send timed out");
                            return;
                        }
                    }
                }
            }
        }
    });
    forwarders.push((task_conn_id, task));
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_auth_metadata_without_consuming_token_protocol() {
        let bytes = b"GET /signal?access_token=query HTTP/1.1\r\nHost: example\r\nSec-WebSocket-Protocol: rvoip.webrtc.v1, token.secret\r\n\r\n";
        let metadata = parse_handshake_metadata(bytes).expect("metadata");
        assert_eq!(metadata.query_token.as_deref(), Some("query"));
        assert_eq!(
            metadata.subprotocols,
            vec!["rvoip.webrtc.v1", "token.secret"]
        );
        assert_eq!(
            select_response_protocol(&metadata.subprotocols, &AnonymousAuth).as_deref(),
            Some("rvoip.webrtc.v1")
        );
    }

    #[test]
    fn private_auth_and_attachment_subprotocols_are_never_echoed() {
        struct PrivateHintAuth;

        #[async_trait::async_trait]
        impl WsAuthHook for PrivateHintAuth {
            async fn authenticate(
                &self,
                _subprotocols: &[String],
                _query_token: Option<&str>,
                _peer_addr: SocketAddr,
            ) -> std::result::Result<AuthContext, AuthRejection> {
                unreachable!()
            }

            fn subprotocol_is_private(&self, value: &str) -> bool {
                value.starts_with("token.") || value.starts_with("bridgefu.attach.")
            }
        }

        let requested = vec![
            "bridgefu.attach.private".into(),
            "token.private".into(),
            "rvoip.webrtc.v1".into(),
        ];
        assert_eq!(
            select_response_protocol(&requested, &PrivateHintAuth).as_deref(),
            Some("rvoip.webrtc.v1")
        );
    }
}
