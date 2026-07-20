//! WebSocket JSON SDP signaler (feature `signaling-ws`).
//!
//! Authentication is completed before the HTTP 101 response for both WS and
//! WSS. Once upgraded, every signaling mutation is authorized against the
//! adapter-owned route identity shared with WHIP/WHEP.

use std::collections::{HashMap, HashSet};
use std::future::Future;
use std::net::SocketAddr;
use std::pin::Pin;
use std::sync::Arc;
use std::task::{Context, Poll};
use std::time::Duration;

use futures::{stream::SplitSink, SinkExt, StreamExt};
use rvoip_core::adapter::{ConnectionAdapter, InboundRoutingHint};
use rvoip_core::ids::ConnectionId;
use serde::{Deserialize, Serialize};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt, ReadBuf};
use tokio::net::TcpListener;
use tokio::sync::{mpsc, Mutex as AsyncMutex};
use tokio::task::JoinSet;
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

use crate::adapter::{RemoteAdmissionOutcome, RouteAuthorization, WebRtcAdapter};
use crate::errors::{Result, WebRtcError};
use crate::peer::LocalIceEvent;
use crate::signaling::auth::{
    AnonymousAuth, AuthContext, AuthRejection, WsAuthHook, SIGNALING_SUBPROTOCOL,
};

const MAX_HANDSHAKE_BYTES: usize = 16 * 1024;
const HANDSHAKE_TIMEOUT: Duration = Duration::from_secs(10);

#[derive(Deserialize, Serialize, Default)]
pub struct SignalingMessage {
    #[serde(rename = "type")]
    pub msg_type: String,
    #[serde(default)]
    pub sdp: String,
    #[serde(default, rename = "connection_id")]
    pub connection_id: String,
    #[serde(default)]
    pub candidate: String,
    /// Client-generated correlation key for one logical signaling request.
    /// Required for new `rvoip.webrtc.v1` offers and echoed in replies.
    #[serde(default, rename = "request_id")]
    pub request_id: String,
}

impl std::fmt::Debug for SignalingMessage {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("SignalingMessage")
            .field("type_present", &!self.msg_type.is_empty())
            .field("sdp_bytes", &self.sdp.len())
            .field("connection_present", &!self.connection_id.is_empty())
            .field("candidate_bytes", &self.candidate.len())
            .field("request_id_present", &!self.request_id.is_empty())
            .finish()
    }
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

/// Adapter lifecycle outcomes serialized by the socket owner. Keeping these
/// on the same loop as SDP, ICE, and BYE preserves frame ordering and binds an
/// application decision to both halves of the signaling identity.
enum RouteLifecycleSignal {
    Ready {
        connection_id: ConnectionId,
        request_id: String,
    },
    Rejected {
        connection_id: ConnectionId,
        request_id: String,
    },
    Terminal {
        connection_id: ConnectionId,
    },
}

struct HandshakeMetadata {
    subprotocols: Vec<String>,
    query_token: Option<String>,
}

impl std::fmt::Debug for HandshakeMetadata {
    fn fmt(&self, formatter: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        formatter
            .debug_struct("HandshakeMetadata")
            .field("subprotocol_count", &self.subprotocols.len())
            .field("query_token_present", &self.query_token.is_some())
            .field(
                "query_token_len",
                &self.query_token.as_deref().map(str::len),
            )
            .finish()
    }
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
    serve_listener_with_auth_and_shutdown(listener, adapter, auth, std::future::pending()).await
}

/// Serve authenticated WS signaling with owned connection tasks.
///
/// When `shutdown` resolves, accepted connection tasks are aborted and joined
/// before the listener future returns. This prevents the common
/// `select!`-drops-the-accept-loop pattern from detaching active sockets.
pub async fn serve_listener_with_auth_and_shutdown(
    listener: TcpListener,
    adapter: Arc<WebRtcAdapter>,
    auth: Arc<dyn WsAuthHook>,
    shutdown: impl Future<Output = ()> + Send,
) -> Result<()> {
    let mut connections = JoinSet::new();
    tokio::pin!(shutdown);
    let result = loop {
        tokio::select! {
            biased;
            _ = &mut shutdown => break Ok(()),
            completed = connections.join_next(), if !connections.is_empty() => {
                if let Some(Err(error)) = completed {
                    warn!(error_class = "ws-connection-task", error = %error, "WS signaling connection task failed");
                }
            }
            accepted = listener.accept() => {
                let (mut stream, peer_addr) = match accepted {
                    Ok(accepted) => accepted,
                    Err(error) => break Err(WebRtcError::Signaling(error.to_string())),
                };
                let Some(task_guard) = adapter.try_start_inbound_ws_connection_task() else {
                    let _ = write_overload_rejection(&mut stream).await;
                    continue;
                };
                let adapter = Arc::clone(&adapter);
                let auth = Arc::clone(&auth);
                connections.spawn(async move {
                    let _task_guard = task_guard;
                    if let Err(error) = handle_authenticated_stream(stream, adapter, auth, peer_addr).await {
                        warn!("ws signaling connection error: {error}");
                    }
                });
            }
        }
    };
    connections.abort_all();
    while let Some(completed) = connections.join_next().await {
        if let Err(error) = completed {
            if !error.is_cancelled() {
                warn!(error_class = "ws-connection-task", error = %error, "WS signaling connection task failed during drain");
            }
        }
    }
    result
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
    serve_tls_listener_with_auth_and_shutdown(listener, tls, adapter, auth, std::future::pending())
        .await
}

#[cfg(feature = "tls-rustls")]
pub async fn serve_tls_listener_with_auth_and_shutdown(
    listener: TcpListener,
    tls: crate::tls::TlsConfig,
    adapter: Arc<WebRtcAdapter>,
    auth: Arc<dyn WsAuthHook>,
    shutdown: impl Future<Output = ()> + Send,
) -> Result<()> {
    let mut connections = JoinSet::new();
    tokio::pin!(shutdown);
    let result = loop {
        tokio::select! {
            biased;
            _ = &mut shutdown => break Ok(()),
            completed = connections.join_next(), if !connections.is_empty() => {
                if let Some(Err(error)) = completed {
                    warn!(error_class = "wss-connection-task", error = %error, "WSS signaling connection task failed");
                }
            }
            accepted = listener.accept() => {
                let (mut stream, peer_addr) = match accepted {
                    Ok(accepted) => accepted,
                    Err(error) => break Err(WebRtcError::Signaling(error.to_string())),
                };
                let Some(task_guard) = adapter.try_start_inbound_ws_connection_task() else {
                    let _ = stream.shutdown().await;
                    continue;
                };
                let acceptor = tls.acceptor.clone();
                let adapter = Arc::clone(&adapter);
                let auth = Arc::clone(&auth);
                connections.spawn(async move {
                    let _task_guard = task_guard;
                    let stream = match acceptor.accept(stream).await {
                        Ok(stream) => stream,
                        Err(_) => {
                            warn!(error_class = "tls-handshake", "WSS TLS handshake failed");
                            return;
                        }
                    };
                    if let Err(error) = handle_authenticated_stream(stream, adapter, auth, peer_addr).await {
                        warn!("wss signaling connection error: {error}");
                    }
                });
            }
        }
    };
    connections.abort_all();
    while let Some(completed) = connections.join_next().await {
        if let Err(error) = completed {
            if !error.is_cancelled() {
                warn!(error_class = "wss-connection-task", error = %error, "WSS signaling connection task failed during drain");
            }
        }
    }
    result
}

async fn write_overload_rejection(stream: &mut tokio::net::TcpStream) -> std::io::Result<()> {
    stream
        .write_all(
            b"HTTP/1.1 503 Service Unavailable\r\nConnection: close\r\nRetry-After: 1\r\nCache-Control: no-store\r\nContent-Length: 0\r\n\r\n",
        )
        .await?;
    stream.shutdown().await
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
    let Some((ws, auth_context, route_leases)) =
        upgrade_with_auth(stream, &adapter, auth.as_ref(), peer_addr).await?
    else {
        return Ok(());
    };
    drive_ws_loop(ws, adapter, auth_context, route_leases).await
}

async fn upgrade_with_auth<S>(
    mut stream: S,
    adapter: &Arc<WebRtcAdapter>,
    auth: &dyn WsAuthHook,
    peer_addr: SocketAddr,
) -> Result<Option<(WebSocketStream<PrefixedStream<S>>, AuthContext, bool)>>
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
    let route_leases = response_protocol.as_deref() == Some(SIGNALING_SUBPROTOCOL);
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
    Ok(Some((ws, auth_context, route_leases)))
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
    let _ = auth;
    subprotocols
        .iter()
        .any(|value| value == SIGNALING_SUBPROTOCOL)
        .then(|| SIGNALING_SUBPROTOCOL.to_owned())
}

fn valid_request_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.is_ascii()
        && value
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'.' | b'_' | b'-' | b':'))
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
    route_leases: bool,
) -> Result<()>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    let authorization = auth_context.route_authorization();
    let mut routing_hint = auth_context.session_hint;
    let (write, mut read) = ws.split();
    let write: WsSink<S> = Arc::new(AsyncMutex::new(write));
    let mut forwarders = Vec::new();
    let mut terminal_forwarders = Vec::new();
    let (terminal_tx, mut terminal_rx) = mpsc::unbounded_channel();
    let mut owned_routes = HashSet::new();
    let mut request_routes = HashMap::new();

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
        loop {
            tokio::select! {
                biased;
                message = read.next() => {
                    let Some(message) = message else { break; };
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
                        &mut routing_hint,
                        &mut forwarders,
                        &mut terminal_forwarders,
                        &terminal_tx,
                        route_leases,
                        &mut owned_routes,
                        &mut request_routes,
                    )
                    .await?;
                    if should_close {
                        break;
                    }
                }
                lifecycle = terminal_rx.recv() => {
                    let Some(lifecycle) = lifecycle else { break; };
                    let (connection_id, request_id, msg_type, terminal) = match lifecycle {
                        RouteLifecycleSignal::Ready { connection_id, request_id } => {
                            (connection_id, request_id, "ready", false)
                        }
                        RouteLifecycleSignal::Rejected { connection_id, request_id } => {
                            (connection_id, request_id, "rejected", true)
                        }
                        RouteLifecycleSignal::Terminal { connection_id } => {
                            (connection_id, String::new(), "bye", true)
                        }
                    };
                    if !owned_routes.contains(&connection_id) {
                        continue;
                    }
                    if msg_type != "bye"
                        && request_routes.get(&request_id) != Some(&connection_id)
                    {
                        // This is an internal invariant failure, not remote
                        // input. Fail the socket closed rather than emit an
                        // outcome under the wrong request authority.
                        return Err(WebRtcError::Signaling(
                            "WebRTC admission outcome ownership mismatch".into(),
                        ));
                    }
                    send_message(
                        &write,
                        &SignalingMessage {
                            msg_type: msg_type.into(),
                            sdp: String::new(),
                            connection_id: connection_id.to_string(),
                            candidate: String::new(),
                            request_id,
                        },
                    )
                    .await?;
                    if !terminal {
                        continue;
                    }
                    owned_routes.remove(&connection_id);
                    request_routes.retain(|_, route| route != &connection_id);
                    if let Some(index) = terminal_forwarders
                        .iter()
                        .position(|(route, _)| route == &connection_id)
                    {
                        let (_, mut task) = terminal_forwarders.swap_remove(index);
                        let _ = (&mut task).await;
                    }
                    if let Some(index) = forwarders
                        .iter()
                        .position(|(route, _)| route == &connection_id)
                    {
                        let (_, mut task) = forwarders.swap_remove(index);
                        task.abort();
                        let _ = (&mut task).await;
                    }
                }
            }
        }
        Ok(())
    }
    .await;

    if let Some(mut task) = keepalive {
        task.abort();
        let _ = (&mut task).await;
    }
    for (_, mut task) in terminal_forwarders {
        task.abort();
        let _ = (&mut task).await;
    }
    if route_leases {
        for connection_id in owned_routes.drain() {
            let _ = adapter
                .end_leased_route_owned(
                    connection_id,
                    rvoip_core::adapter::EndReason::Normal,
                    &authorization,
                )
                .await;
        }
    }
    for (_, mut task) in forwarders {
        task.abort();
        let _ = (&mut task).await;
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
    routing_hint: &mut Option<String>,
    forwarders: &mut Vec<(ConnectionId, tokio::task::JoinHandle<()>)>,
    terminal_forwarders: &mut Vec<(ConnectionId, tokio::task::JoinHandle<()>)>,
    terminal_tx: &mpsc::UnboundedSender<RouteLifecycleSignal>,
    route_leases: bool,
    owned_routes: &mut HashSet<ConnectionId>,
    request_routes: &mut HashMap<String, ConnectionId>,
) -> Result<bool>
where
    S: AsyncRead + AsyncWrite + Unpin + Send + 'static,
{
    match parsed.msg_type.as_str() {
        "offer" | "offer-ready" => {
            // `offer-ready` is an explicit, default-off extension. A legacy
            // client sends `offer`, so a new server must not emit unfamiliar
            // readiness frames onto that socket. Conversely, an older server
            // rejects `offer-ready`, making a required client fail closed.
            let require_ready = parsed.msg_type == "offer-ready";
            let creating = parsed.connection_id.is_empty();
            if require_ready && !creating {
                return Err(WebRtcError::Signaling(
                    "offer-ready is valid only for a new WebRTC route".into(),
                ));
            }
            if require_ready && !route_leases {
                return Err(WebRtcError::Signaling(
                    "offer-ready requires authenticated rvoip.webrtc.v1 route leasing".into(),
                ));
            }
            if require_ready && !authorization.is_authenticated_principal() {
                return Err(WebRtcError::Signaling(
                    "offer-ready requires a non-anonymous authenticated principal".into(),
                ));
            }
            if route_leases && !valid_request_id(&parsed.request_id) {
                return Err(WebRtcError::Signaling(
                    "rvoip.webrtc.v1 offer requires a bounded request_id".into(),
                ));
            }
            if creating
                && !parsed.request_id.is_empty()
                && request_routes.contains_key(&parsed.request_id)
            {
                return Err(WebRtcError::Signaling(
                    "duplicate WebRTC signaling request_id".into(),
                ));
            }
            if creating && require_ready {
                let routing_hint = if adapter.supports_inbound_admission_confirmation() {
                    routing_hint.take()
                } else {
                    routing_hint.clone()
                }
                .map(InboundRoutingHint::new)
                .transpose()
                .map_err(|_| {
                    WebRtcError::Signaling("authenticated WebSocket session hint is invalid".into())
                })?;
                let prepared = adapter
                    .prepare_offer_ready_authorized_with_hint(
                        &parsed.sdp,
                        authorization.clone(),
                        routing_hint,
                    )
                    .await?;
                let conn_id = prepared.connection_id().clone();
                let answer = prepared.answer_sdp().to_owned();

                // Lease and subscribe before either the answer or inbound
                // publication becomes visible. An immediate core accept or
                // reject is therefore retained under this exact request id.
                owned_routes.insert(conn_id.clone());
                request_routes.insert(parsed.request_id.clone(), conn_id.clone());
                ensure_route_terminal_forwarder(
                    adapter,
                    &conn_id,
                    Some(parsed.request_id.clone()),
                    terminal_tx,
                    terminal_forwarders,
                );
                send_message(
                    write,
                    &SignalingMessage {
                        msg_type: "answer".into(),
                        sdp: answer,
                        connection_id: conn_id.to_string(),
                        candidate: String::new(),
                        request_id: parsed.request_id,
                    },
                )
                .await?;
                if adapter.trickle_ice_enabled() {
                    ensure_local_ice_forwarder(adapter, &conn_id, write, forwarders);
                }
                adapter.publish_prepared_offer_ready(prepared).await?;
                return Ok(false);
            }
            let (conn_id, answer) = if creating {
                let routing_hint = if adapter.supports_inbound_admission_confirmation() {
                    routing_hint.take()
                } else {
                    routing_hint.clone()
                }
                .map(InboundRoutingHint::new)
                .transpose()
                .map_err(|_| {
                    WebRtcError::Signaling("authenticated WebSocket session hint is invalid".into())
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
                if route_leases && !owned_routes.contains(&conn_id) {
                    return Err(WebRtcError::Signaling(
                        "WebRTC renegotiation route is not leased to this socket".into(),
                    ));
                }
                let answer = adapter
                    .apply_ice_restart_offer_authorized(conn_id.clone(), &parsed.sdp, authorization)
                    .await?;
                (conn_id, answer)
            };
            if route_leases && creating {
                owned_routes.insert(conn_id.clone());
                request_routes.insert(parsed.request_id.clone(), conn_id.clone());
            }
            if route_leases {
                ensure_route_terminal_forwarder(
                    adapter,
                    &conn_id,
                    require_ready.then(|| parsed.request_id.clone()),
                    terminal_tx,
                    terminal_forwarders,
                );
            }
            send_message(
                write,
                &SignalingMessage {
                    msg_type: "answer".into(),
                    sdp: answer,
                    connection_id: conn_id.to_string(),
                    candidate: String::new(),
                    request_id: parsed.request_id,
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
            if route_leases {
                owned_routes.insert(conn_id.clone());
                ensure_route_terminal_forwarder(
                    adapter,
                    &conn_id,
                    None,
                    terminal_tx,
                    terminal_forwarders,
                );
            }
            send_message(
                write,
                &SignalingMessage {
                    msg_type: "ack".into(),
                    sdp: String::new(),
                    connection_id: parsed.connection_id,
                    candidate: String::new(),
                    request_id: parsed.request_id,
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
            if route_leases && !owned_routes.contains(&conn_id) {
                return Err(WebRtcError::Signaling(
                    "WebRTC ICE route is not leased to this socket".into(),
                ));
            }
            let candidate: RTCIceCandidateInit = serde_json::from_str(&parsed.candidate)
                .map_err(|error| WebRtcError::Signaling(format!("ice-candidate parse: {error}")))?;
            adapter
                .apply_trickle_candidate_authorized(&conn_id, candidate, authorization)
                .await?;
        }
        "ice-complete" => {
            if parsed.connection_id.is_empty() || !parsed.candidate.is_empty() {
                return Err(WebRtcError::Signaling(
                    "ice-complete requires only connection_id".into(),
                ));
            }
            let conn_id = ConnectionId::from_string(parsed.connection_id);
            if route_leases && !owned_routes.contains(&conn_id) {
                return Err(WebRtcError::Signaling(
                    "WebRTC ICE route is not leased to this socket".into(),
                ));
            }
            adapter
                .apply_trickle_candidate_authorized(
                    &conn_id,
                    RTCIceCandidateInit::default(),
                    authorization,
                )
                .await?;
        }
        "bye" => {
            if parsed.connection_id.is_empty() {
                return Ok(!route_leases);
            }
            let connection_id = ConnectionId::from_string(parsed.connection_id);
            if route_leases && !owned_routes.contains(&connection_id) {
                return Err(WebRtcError::Signaling(
                    "WebRTC BYE route is not leased to this socket".into(),
                ));
            }
            if let Some(index) = terminal_forwarders
                .iter()
                .position(|(route, _)| route == &connection_id)
            {
                let (_, mut task) = terminal_forwarders.swap_remove(index);
                task.abort();
                let _ = (&mut task).await;
            }
            if route_leases {
                adapter
                    .end_leased_route_owned(
                        connection_id.clone(),
                        rvoip_core::adapter::EndReason::Normal,
                        authorization,
                    )
                    .await?;
            } else {
                adapter
                    .end_authorized(
                        connection_id.clone(),
                        rvoip_core::adapter::EndReason::Normal,
                        authorization,
                    )
                    .await?;
            }
            owned_routes.remove(&connection_id);
            request_routes.retain(|_, route| route != &connection_id);
            if let Some(index) = forwarders
                .iter()
                .position(|(route, _)| route == &connection_id)
            {
                let (_, mut task) = forwarders.swap_remove(index);
                task.abort();
                let _ = (&mut task).await;
            }
            return Ok(!route_leases);
        }
        other => {
            return Err(WebRtcError::Signaling(format!(
                "unknown signaling message type: {other}"
            )));
        }
    }
    Ok(false)
}

fn ensure_route_terminal_forwarder(
    adapter: &Arc<WebRtcAdapter>,
    conn_id: &ConnectionId,
    request_id: Option<String>,
    terminal_tx: &mpsc::UnboundedSender<RouteLifecycleSignal>,
    forwarders: &mut Vec<(ConnectionId, tokio::task::JoinHandle<()>)>,
) {
    if let Some(index) = forwarders
        .iter()
        .position(|(existing, _)| existing == conn_id)
    {
        if !forwarders[index].1.is_finished() {
            return;
        }
        forwarders.swap_remove(index);
    }
    let Some((mut cancellation, mut admission)) = adapter.routes().get(conn_id).map(|route| {
        (
            route.subscribe_cancellation(),
            route.subscribe_remote_admission(),
        )
    }) else {
        return;
    };
    let conn_id = conn_id.clone();
    let task_conn_id = conn_id.clone();
    let terminal_tx = terminal_tx.clone();
    let task = tokio::spawn(async move {
        let mut readiness_sent = false;
        loop {
            match *admission.borrow_and_update() {
                RemoteAdmissionOutcome::Accepted if !readiness_sent && request_id.is_some() => {
                    readiness_sent = true;
                    if terminal_tx
                        .send(RouteLifecycleSignal::Ready {
                            connection_id: conn_id.clone(),
                            request_id: request_id.clone().expect("checked above"),
                        })
                        .is_err()
                    {
                        return;
                    }
                }
                RemoteAdmissionOutcome::Rejected if request_id.is_some() => {
                    let _ = terminal_tx.send(RouteLifecycleSignal::Rejected {
                        connection_id: conn_id,
                        request_id: request_id.expect("checked above"),
                    });
                    return;
                }
                RemoteAdmissionOutcome::Pending
                | RemoteAdmissionOutcome::Accepted
                | RemoteAdmissionOutcome::Rejected => {}
            }
            if *cancellation.borrow_and_update() {
                let _ = terminal_tx.send(RouteLifecycleSignal::Terminal {
                    connection_id: conn_id,
                });
                return;
            }
            tokio::select! {
                biased;
                changed = admission.changed() => {
                    if changed.is_err() {
                        return;
                    }
                }
                changed = cancellation.changed() => {
                    if changed.is_err() {
                        return;
                    }
                }
            }
        }
    });
    forwarders.push((task_conn_id, task));
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
                event = route.peer.recv_local_ice_event() => {
                    let Some(event) = event else { return; };
                    let complete = matches!(&event, LocalIceEvent::Complete);
                    let message = match event {
                        LocalIceEvent::Candidate(candidate) => {
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
                            SignalingMessage {
                                msg_type: "ice-candidate".into(),
                                sdp: String::new(),
                                connection_id: conn_id.to_string(),
                                candidate: payload,
                                request_id: String::new(),
                            }
                        }
                        LocalIceEvent::Complete => SignalingMessage {
                            msg_type: "ice-complete".into(),
                            sdp: String::new(),
                            connection_id: conn_id.to_string(),
                            candidate: String::new(),
                            request_id: String::new(),
                        },
                        LocalIceEvent::Overflow => {
                            warn!(connection_id = %conn_id, "local ICE signaling queue overflowed");
                            let _ = rvoip_core::adapter::ConnectionAdapter::end(
                                &*adapter,
                                conn_id.clone(),
                                rvoip_core::adapter::EndReason::Failed {
                                    detail: "local ICE signaling queue overflowed".into(),
                                },
                            )
                            .await;
                            return;
                        }
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
                    if complete {
                        return;
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
        assert_eq!(
            select_response_protocol(
                &["bridgefu.attach.private".into(), "unknown.protocol".into()],
                &PrivateHintAuth,
            ),
            None,
            "private and unknown protocols must never be echoed"
        );
    }

    #[test]
    fn handshake_metadata_debug_redacts_query_tokens_and_subprotocol_values() {
        const CANARY: &str = "ws-query-token-canary";
        let metadata = HandshakeMetadata {
            subprotocols: vec![CANARY.into()],
            query_token: Some(CANARY.into()),
        };
        let rendered = format!("{metadata:?}");
        assert!(!rendered.contains(CANARY), "credential leaked: {rendered}");
        assert_eq!(metadata.query_token.as_deref(), Some(CANARY));
    }

    #[test]
    fn v1_request_ids_are_bounded_ascii_tokens() {
        assert!(valid_request_id("route-123:offer_1"));
        assert!(!valid_request_id(""));
        assert!(!valid_request_id("contains space"));
        assert!(!valid_request_id("contains/route"));
        assert!(!valid_request_id(&"a".repeat(129)));
    }
}
