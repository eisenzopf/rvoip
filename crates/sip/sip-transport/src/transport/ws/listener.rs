use futures_util::stream::SplitStream;
use futures_util::StreamExt;
#[cfg(feature = "ws")]
use http::HeaderValue;
use std::future::Future;
#[cfg(feature = "ws")]
use std::io;
use std::net::SocketAddr;
use std::sync::Arc;
#[cfg(feature = "ws")]
use std::time::Duration;
use tokio::net::{TcpListener, TcpStream};
use tokio::sync::Semaphore;
#[cfg(feature = "ws")]
use tokio::task::JoinSet;
#[cfg(feature = "ws")]
use tokio_tungstenite::tungstenite::handshake::server::{ErrorResponse, Request, Response};
#[cfg(feature = "ws")]
use tokio_tungstenite::{tungstenite, WebSocketStream};
#[cfg(feature = "ws")]
use tracing::warn;
use tracing::{debug, error, info};

#[cfg(feature = "wss")]
use tokio_rustls::TlsAcceptor;

use super::connection::{sip_websocket_config, WebSocketConnection};
use super::{SipWsStream, SIP_WS_SUBPROTOCOL};
use crate::error::{Error, Result};
use crate::transport::runtime::ConnectionLifecycleConfig;
#[cfg(feature = "wss")]
use crate::transport::tls::TlsServerClientAuthConfig;
use crate::transport::HandshakeAdmissionConfig;

#[cfg(feature = "ws")]
fn select_sip_subprotocol(offered: Option<&str>, _secure: bool) -> Option<&'static str> {
    let required = SIP_WS_SUBPROTOCOL;
    offered
        .map(|value| value.split(',').map(str::trim).any(|item| item == required))
        .is_some_and(|matched| matched)
        .then_some(required)
}

#[cfg(feature = "ws")]
const ACCEPT_RETRY_INITIAL_BACKOFF: Duration = Duration::from_millis(10);
#[cfg(feature = "ws")]
const ACCEPT_RETRY_MAX_BACKOFF: Duration = Duration::from_secs(1);

#[cfg(feature = "ws")]
fn is_recoverable_accept_error(_error: &io::Error) -> bool {
    // A successfully bound listener has no per-error signal that justifies
    // tearing down unrelated established sessions. Even a persistent kernel
    // failure is safer as a bounded-backoff readiness failure; explicit
    // listener shutdown is performed by cancelling the supervisor.
    true
}

/// WebSocket listener for accepting SIP WebSocket connections
pub struct WebSocketListener {
    /// The underlying TCP listener
    listener: TcpListener,
    /// Whether this is a secure WebSocket listener (WSS)
    secure: bool,
    /// TLS certificate path if secure. Captured at bind so a future
    /// hot-reload / diagnostics path can surface the configured
    /// material without re-threading it.
    #[allow(dead_code)]
    cert_path: Option<String>,
    /// TLS key path if secure. See `cert_path` for retention rationale.
    #[allow(dead_code)]
    key_path: Option<String>,
    /// Pre-built TLS acceptor for WSS. Built once at `bind()` time so
    /// every accept() reuses the same `ServerConfig` (and therefore
    /// the same cert chain + session resumption cache).
    #[cfg(feature = "wss")]
    tls_acceptor: Option<TlsAcceptor>,
    /// Admission and complete TLS/HTTP deadline used by the supervised public
    /// server and the transport's internal accept loop.
    handshake_admission: HandshakeAdmissionConfig,
    handshake_semaphore: Arc<Semaphore>,
    established_semaphore: Arc<Semaphore>,
}

impl WebSocketListener {
    /// Binds a WebSocket listener to the specified address.
    ///
    /// For WSS (`secure = true`), `cert_path` and `key_path` must both
    /// be supplied — the same PEM cert / PKCS#8 key shape used by the
    /// TLS transport. The `TlsAcceptor` is built once here so per-
    /// accept handshakes don't re-parse cert material.
    pub async fn bind(
        addr: SocketAddr,
        secure: bool,
        cert_path: Option<&str>,
        key_path: Option<&str>,
    ) -> Result<Self> {
        Self::bind_with_handshake_config(
            addr,
            secure,
            cert_path,
            key_path,
            HandshakeAdmissionConfig::default(),
        )
        .await
    }

    /// Bind with explicit admission and an end-to-end TLS/HTTP upgrade
    /// deadline for supervised sessions.
    pub async fn bind_with_handshake_config(
        addr: SocketAddr,
        secure: bool,
        cert_path: Option<&str>,
        key_path: Option<&str>,
        handshake_admission: HandshakeAdmissionConfig,
    ) -> Result<Self> {
        #[cfg(feature = "wss")]
        {
            return Self::bind_with_client_auth_and_handshake(
                addr,
                secure,
                cert_path,
                key_path,
                TlsServerClientAuthConfig::default(),
                handshake_admission,
            )
            .await;
        }
        #[cfg(not(feature = "wss"))]
        {
            Self::bind_inner(addr, secure, cert_path, key_path, handshake_admission).await
        }
    }

    /// Bind a WebSocket listener with an explicit inbound WSS
    /// client-certificate policy. Plain WS ignores this configuration.
    #[cfg(feature = "wss")]
    pub async fn bind_with_client_auth(
        addr: SocketAddr,
        secure: bool,
        cert_path: Option<&str>,
        key_path: Option<&str>,
        client_auth: TlsServerClientAuthConfig,
    ) -> Result<Self> {
        Self::bind_with_client_auth_and_handshake(
            addr,
            secure,
            cert_path,
            key_path,
            client_auth,
            HandshakeAdmissionConfig::default(),
        )
        .await
    }

    /// Bind with both WSS client authentication and supervised admission.
    #[cfg(feature = "wss")]
    pub async fn bind_with_client_auth_and_handshake(
        addr: SocketAddr,
        secure: bool,
        cert_path: Option<&str>,
        key_path: Option<&str>,
        client_auth: TlsServerClientAuthConfig,
        handshake_admission: HandshakeAdmissionConfig,
    ) -> Result<Self> {
        Self::bind_inner_with_client_auth(
            addr,
            secure,
            cert_path,
            key_path,
            client_auth,
            handshake_admission,
        )
        .await
    }

    #[cfg(feature = "wss")]
    async fn bind_inner_with_client_auth(
        addr: SocketAddr,
        secure: bool,
        cert_path: Option<&str>,
        key_path: Option<&str>,
        client_auth: TlsServerClientAuthConfig,
        handshake_admission: HandshakeAdmissionConfig,
    ) -> Result<Self> {
        Self::bind_inner_impl(
            addr,
            secure,
            cert_path,
            key_path,
            Some(client_auth),
            handshake_admission,
        )
        .await
    }

    #[cfg(not(feature = "wss"))]
    async fn bind_inner(
        addr: SocketAddr,
        secure: bool,
        cert_path: Option<&str>,
        key_path: Option<&str>,
        handshake_admission: HandshakeAdmissionConfig,
    ) -> Result<Self> {
        Self::bind_inner_impl(addr, secure, cert_path, key_path, handshake_admission).await
    }

    #[cfg(feature = "wss")]
    async fn bind_inner_impl(
        addr: SocketAddr,
        secure: bool,
        cert_path: Option<&str>,
        key_path: Option<&str>,
        client_auth: Option<TlsServerClientAuthConfig>,
        handshake_admission: HandshakeAdmissionConfig,
    ) -> Result<Self> {
        let client_auth = client_auth.unwrap_or_default();
        Self::bind_configured(
            addr,
            secure,
            cert_path,
            key_path,
            &client_auth,
            handshake_admission,
        )
        .await
    }

    #[cfg(not(feature = "wss"))]
    async fn bind_inner_impl(
        addr: SocketAddr,
        secure: bool,
        cert_path: Option<&str>,
        key_path: Option<&str>,
        handshake_admission: HandshakeAdmissionConfig,
    ) -> Result<Self> {
        Self::bind_configured(addr, secure, cert_path, key_path, handshake_admission).await
    }

    #[cfg(feature = "wss")]
    async fn bind_configured(
        addr: SocketAddr,
        secure: bool,
        cert_path: Option<&str>,
        key_path: Option<&str>,
        client_auth: &TlsServerClientAuthConfig,
        handshake_admission: HandshakeAdmissionConfig,
    ) -> Result<Self> {
        let handshake_admission =
            handshake_admission.validate(if secure { "WSS" } else { "WS" })?;
        let listener = TcpListener::bind(addr)
            .await
            .map_err(|e| Error::BindFailed(addr, e))?;

        info!(
            local_addr = %listener.local_addr().unwrap(),
            transport = if secure { "wss" } else { "ws" },
            client_auth_mode = ?client_auth.mode,
            "WebSocket listener bound"
        );

        // Build the TLS acceptor up-front when the listener is secure.
        // Reuses the TLS transport's PEM loaders (re-exported as
        // `pub(crate)`) so cert/key handling is identical to the TLS
        // path — same root config, same error surface.
        #[cfg(feature = "wss")]
        let tls_acceptor = if secure {
            let (cert_p, key_p) = match (cert_path, key_path) {
                (Some(c), Some(k)) => (c, k),
                _ => {
                    return Err(Error::TlsHandshakeFailed(
                        "WSS listener requires both cert_path and key_path".into(),
                    ))
                }
            };
            let server_config = crate::transport::tls::build_server_config(
                std::path::Path::new(cert_p),
                std::path::Path::new(key_p),
                client_auth,
                "WSS",
            )?;
            Some(TlsAcceptor::from(Arc::new(server_config)))
        } else {
            None
        };

        Ok(Self {
            listener,
            secure,
            cert_path: cert_path.map(String::from),
            key_path: key_path.map(String::from),
            #[cfg(feature = "wss")]
            tls_acceptor,
            handshake_admission,
            handshake_semaphore: Arc::new(Semaphore::new(handshake_admission.max_concurrent)),
            established_semaphore: Arc::new(Semaphore::new(
                ConnectionLifecycleConfig::from_handshake(handshake_admission)
                    .max_established_per_direction,
            )),
        })
    }

    #[cfg(not(feature = "wss"))]
    async fn bind_configured(
        addr: SocketAddr,
        secure: bool,
        cert_path: Option<&str>,
        key_path: Option<&str>,
        handshake_admission: HandshakeAdmissionConfig,
    ) -> Result<Self> {
        let handshake_admission =
            handshake_admission.validate(if secure { "WSS" } else { "WS" })?;
        let listener = TcpListener::bind(addr)
            .await
            .map_err(|e| Error::BindFailed(addr, e))?;
        info!(
            local_addr = %listener.local_addr().unwrap(),
            transport = if secure { "wss" } else { "ws" },
            "WebSocket listener bound"
        );
        Ok(Self {
            listener,
            secure,
            cert_path: cert_path.map(String::from),
            key_path: key_path.map(String::from),
            handshake_admission,
            handshake_semaphore: Arc::new(Semaphore::new(handshake_admission.max_concurrent)),
            established_semaphore: Arc::new(Semaphore::new(
                ConnectionLifecycleConfig::from_handshake(handshake_admission)
                    .max_established_per_direction,
            )),
        })
    }

    /// Returns the local address this listener is bound to
    pub fn local_addr(&self) -> Result<SocketAddr> {
        self.listener
            .local_addr()
            .map_err(|e| Error::LocalAddrFailed(e))
    }

    /// Concurrent, supervised public listener API.
    ///
    /// Raw sockets are accepted continuously and each TLS/HTTP upgrade runs in
    /// a separately supervised task under the configured deadline. Handshake
    /// and established-session permits are distinct; dropping this future
    /// aborts every child task through `JoinSet` ownership.
    #[cfg(feature = "ws")]
    pub async fn serve_concurrent<F, Fut>(self: Arc<Self>, handler: F) -> Result<()>
    where
        F: Fn(Arc<WebSocketConnection>, SplitStream<WebSocketStream<SipWsStream>>) -> Fut
            + Send
            + Sync
            + 'static,
        Fut: Future<Output = ()> + Send + 'static,
    {
        let handler = Arc::new(handler);
        let lifecycle = ConnectionLifecycleConfig::from_handshake(self.handshake_admission);
        let mut tasks = JoinSet::new();
        let mut accept_retry_backoff = ACCEPT_RETRY_INITIAL_BACKOFF;
        loop {
            while let Some(completed) = tasks.try_join_next() {
                if let Err(error) = completed {
                    if !error.is_cancelled() {
                        error!("supervised WebSocket connection task failed: {error}");
                    }
                }
            }
            let handshake_permit = self
                .handshake_semaphore
                .clone()
                .acquire_owned()
                .await
                .map_err(|_| Error::TransportClosed)?;
            let (stream, peer_addr) = match self.accept_tcp().await {
                Ok(accepted) => {
                    accept_retry_backoff = ACCEPT_RETRY_INITIAL_BACKOFF;
                    accepted
                }
                Err(Error::ReceiveFailed(error)) if is_recoverable_accept_error(&error) => {
                    warn!(
                        error_kind = ?error.kind(),
                        retry_ms = accept_retry_backoff.as_millis(),
                        "recoverable WebSocket accept failure; preserving live sessions and retrying"
                    );
                    drop(handshake_permit);
                    tokio::time::sleep(accept_retry_backoff).await;
                    accept_retry_backoff = accept_retry_backoff
                        .saturating_mul(2)
                        .min(ACCEPT_RETRY_MAX_BACKOFF);
                    continue;
                }
                Err(error) => return Err(error),
            };
            let listener = self.clone();
            let handler = handler.clone();
            tasks.spawn(async move {
                let upgraded = tokio::time::timeout(
                    listener.handshake_admission.timeout,
                    listener.upgrade_tcp(stream, peer_addr),
                )
                .await;
                drop(handshake_permit);
                let (connection, reader) = match upgraded {
                    Ok(Ok(connection)) => connection,
                    Ok(Err(error)) => {
                        debug!(source = %peer_addr, "supervised WebSocket upgrade rejected: {error}");
                        return;
                    }
                    Err(_) => {
                        debug!(source = %peer_addr, "supervised WebSocket upgrade timed out");
                        return;
                    }
                };
                let established_permit =
                    match listener.established_semaphore.clone().try_acquire_owned() {
                        Ok(permit) => permit,
                        Err(_) => {
                            let _ = connection.close().await;
                            return;
                        }
                    };
                let connection = Arc::new(connection);
                let mut activity = connection.activity_receiver();
                let mut writer_closed = connection.writer_closed_receiver();
                let established_at = tokio::time::Instant::now();
                let handler = handler(connection.clone(), reader);
                tokio::pin!(handler);
                loop {
                    if *writer_closed.borrow() {
                        break;
                    }
                    let deadline =
                        lifecycle.next_deadline(*activity.borrow(), established_at);
                    tokio::select! {
                        _ = &mut handler => break,
                        _ = writer_closed.changed() => break,
                        changed = activity.changed() => {
                            if changed.is_err() {
                                break;
                            }
                        }
                        _ = tokio::time::sleep_until(deadline) => break,
                    }
                }
                let _ = connection.close().await;
                drop(established_permit);
            });
        }
    }

    /// Accept one TCP socket without performing TLS or HTTP/WebSocket work.
    ///
    /// `WebSocketTransport` uses this split boundary so a slow peer cannot hold
    /// the only listener accept loop while it waits for a ClientHello or HTTP
    /// upgrade request.
    #[cfg(feature = "ws")]
    pub(crate) async fn accept_tcp(&self) -> Result<(TcpStream, SocketAddr)> {
        self.listener
            .accept()
            .await
            .map_err(|error| Error::ReceiveFailed(error))
    }

    /// Complete the WSS TLS handshake, when enabled, and the RFC 7118 HTTP
    /// upgrade on an already accepted TCP socket.
    ///
    /// Callers must apply a deadline and admission limit around this entire
    /// future. Keeping both phases here preserves the verified TLS metadata and
    /// exact `sip` subprotocol boundary.
    #[cfg(feature = "ws")]
    pub(crate) async fn upgrade_tcp(
        &self,
        stream: TcpStream,
        peer_addr: SocketAddr,
    ) -> Result<(
        WebSocketConnection,
        SplitStream<WebSocketStream<SipWsStream>>,
    )> {
        debug!("Accepted TCP connection for WebSocket from {}", peer_addr);

        // For WSS, run the TLS handshake on the freshly accepted TCP
        // socket BEFORE handing it to `accept_async`. The resulting
        // server-side `TlsStream` is wrapped in `SipWsStream::ServerTls`
        // (the client-side `MaybeTlsStream::Rustls` variant doesn't
        // cover the server direction). Plain WS skips the handshake and
        // wraps the raw TCP stream as `SipWsStream::Plain`.
        let (maybe_tls_stream, connection_metadata): (SipWsStream, _) = if self.secure {
            #[cfg(feature = "wss")]
            {
                let acceptor = self.tls_acceptor.as_ref().ok_or_else(|| {
                    Error::TlsHandshakeFailed(
                        "WSS listener marked secure but no TLS acceptor configured".into(),
                    )
                })?;
                let tls_stream = acceptor.accept(stream).await.map_err(|error| {
                    let error_class = crate::transport::tls::tls_runtime_failure_class(&error);
                    error!(
                        source = %peer_addr,
                        error_class,
                        "WSS TLS handshake failed"
                    );
                    crate::transport::tls::classify_tls_runtime_error(
                        error,
                        format!("WSS server TLS handshake failed for {peer_addr}"),
                    )
                })?;
                let metadata = crate::transport::tls::verified_peer_metadata(
                    tls_stream.get_ref().1.peer_certificates(),
                );
                (SipWsStream::ServerTls(tls_stream), metadata)
            }
            #[cfg(not(feature = "wss"))]
            {
                return Err(Error::NotImplemented(
                    "WSS listener requires the `wss` feature (rustls plumbing)".into(),
                ));
            }
        } else {
            (SipWsStream::Plain(stream), None)
        };

        // Custom callback for WebSocket handshake to handle subprotocol negotiation
        let callback = |request: &Request, response: Response| {
            // Check for subprotocol request
            let offered = request
                .headers()
                .get("Sec-WebSocket-Protocol")
                .and_then(|h| h.to_str().ok());

            let Some(required_protocol) = select_sip_subprotocol(offered, self.secure) else {
                let rejection: ErrorResponse = http::Response::builder()
                    .status(http::StatusCode::BAD_REQUEST)
                    .body(Some(
                        "required SIP WebSocket subprotocol was not offered".to_string(),
                    ))
                    .expect("static WebSocket rejection response is valid");
                return Err(rejection);
            };

            let mut response = response;
            response.headers_mut().append(
                "Sec-WebSocket-Protocol",
                HeaderValue::from_static(required_protocol),
            );
            Ok((response, required_protocol.to_string()))
        };

        // Perform WebSocket handshake
        let (ws_stream, selected_protocol) =
            Self::accept_async_with_subprotocol(maybe_tls_stream, callback)
                .await
                .map_err(|_error| {
                    error!(
                        source = %peer_addr,
                        error_class = "websocket_handshake_failed",
                        "WebSocket handshake failed"
                    );
                    Error::WebSocketHandshakeFailed(format!(
                        "WebSocket server handshake failed for {peer_addr}"
                    ))
                })?;

        let subprotocol = selected_protocol;

        // Split the stream for separate reading and writing
        let (ws_writer, ws_reader) = ws_stream.split();

        // Create a WebSocket connection
        let lifecycle = ConnectionLifecycleConfig::from_handshake(self.handshake_admission);
        let connection = WebSocketConnection::from_writer_with_runtime(
            ws_writer,
            peer_addr,
            self.secure,
            subprotocol,
            connection_metadata,
            lifecycle.writer_queue_capacity,
            lifecycle.write_timeout,
        );

        Ok((connection, ws_reader))
    }

    /// Helper to accept WebSocket connections with subprotocol selection
    #[cfg(feature = "ws")]
    async fn accept_async_with_subprotocol<F>(
        stream: SipWsStream,
        callback: F,
    ) -> std::result::Result<(WebSocketStream<SipWsStream>, String), tungstenite::Error>
    where
        F: FnOnce(&Request, Response) -> std::result::Result<(Response, String), ErrorResponse>
            + Unpin,
    {
        let selected_protocol = Arc::new(std::sync::Mutex::new(String::new()));
        let selected_for_callback = Arc::clone(&selected_protocol);
        let ws_stream = tokio_tungstenite::accept_hdr_async_with_config(
            stream,
            move |request: &Request, response: Response| {
                let (response, selected) = callback(request, response)?;
                *selected_for_callback
                    .lock()
                    .unwrap_or_else(|poisoned| poisoned.into_inner()) = selected;
                Ok(response)
            },
            Some(sip_websocket_config()),
        )
        .await?;
        let selected = selected_protocol
            .lock()
            .unwrap_or_else(|poisoned| poisoned.into_inner())
            .clone();
        Ok((ws_stream, selected))
    }

    /// Returns whether this is a secure WebSocket listener
    pub fn is_secure(&self) -> bool {
        self.secure
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "ws")]
    #[test]
    fn sip_subprotocol_selection_is_fail_closed() {
        assert_eq!(select_sip_subprotocol(None, false), None);
        assert_eq!(select_sip_subprotocol(Some("chat"), false), None);
        assert_eq!(select_sip_subprotocol(Some("sips"), false), None);
        assert_eq!(
            select_sip_subprotocol(Some("chat, sip"), false),
            Some("sip")
        );
        assert_eq!(select_sip_subprotocol(Some("sips"), true), None);
        assert_eq!(select_sip_subprotocol(Some("chat, sip"), true), Some("sip"));
    }

    #[cfg(feature = "ws")]
    #[test]
    fn accept_retry_policy_preserves_sessions_for_all_kernel_failures() {
        for kind in [
            io::ErrorKind::Interrupted,
            io::ErrorKind::WouldBlock,
            io::ErrorKind::ConnectionAborted,
            io::ErrorKind::ConnectionReset,
            io::ErrorKind::Other,
        ] {
            assert!(
                is_recoverable_accept_error(&io::Error::from(kind)),
                "{kind:?} should retain live sessions and retry accept"
            );
        }
        for kind in [
            io::ErrorKind::InvalidInput,
            io::ErrorKind::PermissionDenied,
            io::ErrorKind::Unsupported,
            io::ErrorKind::NotConnected,
        ] {
            assert!(
                is_recoverable_accept_error(&io::Error::from(kind)),
                "{kind:?} should retain live sessions under bounded retry"
            );
        }
    }

    /// Test binding a WebSocket listener
    #[tokio::test]
    async fn test_websocket_listener_bind() {
        let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let listener = WebSocketListener::bind(addr, false, None, None)
            .await
            .unwrap();

        let bound_addr = listener.local_addr().unwrap();
        assert!(bound_addr.port() > 0); // Random port assigned
        assert_eq!(bound_addr.ip(), addr.ip());
        assert!(!listener.is_secure());
    }

    #[cfg(feature = "ws")]
    #[tokio::test]
    async fn public_supervisor_accepts_valid_peer_while_slow_peer_is_pending() {
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;

        let listener = Arc::new(
            WebSocketListener::bind_with_handshake_config(
                "127.0.0.1:0".parse().unwrap(),
                false,
                None,
                None,
                HandshakeAdmissionConfig::new(std::time::Duration::from_secs(1), 2),
            )
            .await
            .unwrap(),
        );
        let destination = listener.local_addr().unwrap();
        let (accepted_tx, mut accepted_rx) = tokio::sync::mpsc::channel(1);
        let supervisor = {
            let listener = listener.clone();
            tokio::spawn(async move {
                listener
                    .serve_concurrent(move |connection, _reader| {
                        let accepted_tx = accepted_tx.clone();
                        async move {
                            let _ = accepted_tx.send(connection.peer_addr()).await;
                            let _ = connection.close().await;
                        }
                    })
                    .await
            })
        };

        let stalled = TcpStream::connect(destination).await.unwrap();
        let mut request = format!("ws://{destination}/")
            .into_client_request()
            .unwrap();
        request.headers_mut().insert(
            "Sec-WebSocket-Protocol",
            http::HeaderValue::from_static("sip"),
        );
        let (client, _) = tokio_tungstenite::connect_async(request).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_millis(250), accepted_rx.recv())
            .await
            .expect("valid peer was serialized behind Slowloris socket")
            .expect("supervisor stopped before dispatch");

        drop(client);
        drop(stalled);
        supervisor.abort();
        let _ = supervisor.await;
    }

    #[cfg(feature = "ws")]
    #[tokio::test]
    async fn public_supervisor_drops_idle_reader_and_releases_established_capacity() {
        use futures_util::StreamExt as _;
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;

        let listener = Arc::new(
            WebSocketListener::bind_with_handshake_config(
                "127.0.0.1:0".parse().unwrap(),
                false,
                None,
                None,
                HandshakeAdmissionConfig::new(std::time::Duration::from_millis(50), 1),
            )
            .await
            .unwrap(),
        );
        let destination = listener.local_addr().unwrap();
        let (accepted_tx, mut accepted_rx) = tokio::sync::mpsc::channel(2);
        let supervisor = {
            let listener = listener.clone();
            tokio::spawn(async move {
                listener
                    .serve_concurrent(move |connection, _reader| {
                        let accepted_tx = accepted_tx.clone();
                        async move {
                            let _ = accepted_tx.send(connection.peer_addr()).await;
                            std::future::pending::<()>().await;
                        }
                    })
                    .await
            })
        };
        let request = || {
            let mut request = format!("ws://{destination}/")
                .into_client_request()
                .unwrap();
            request.headers_mut().insert(
                "Sec-WebSocket-Protocol",
                http::HeaderValue::from_static("sip"),
            );
            request
        };

        let (mut first_client, _) = tokio_tungstenite::connect_async(request()).await.unwrap();
        accepted_rx.recv().await.expect("first handler not started");
        tokio::time::timeout(std::time::Duration::from_secs(2), first_client.next())
            .await
            .expect("idle supervised reader/socket remained live");

        let (second_client, _) = tokio_tungstenite::connect_async(request()).await.unwrap();
        tokio::time::timeout(std::time::Duration::from_millis(250), accepted_rx.recv())
            .await
            .expect("idle session retained established capacity")
            .expect("supervisor stopped before second handler");

        drop(second_client);
        supervisor.abort();
        let _ = supervisor.await;
    }

    #[cfg(feature = "ws")]
    #[tokio::test]
    async fn public_supervisor_releases_capacity_promptly_after_peer_close() {
        use futures_util::{SinkExt as _, StreamExt as _};
        use tokio_tungstenite::tungstenite::client::IntoClientRequest;
        use tokio_tungstenite::tungstenite::Message as WsMessage;

        let listener = Arc::new(
            WebSocketListener::bind_with_handshake_config(
                "127.0.0.1:0".parse().unwrap(),
                false,
                None,
                None,
                HandshakeAdmissionConfig::new(std::time::Duration::from_secs(2), 1),
            )
            .await
            .unwrap(),
        );
        let destination = listener.local_addr().unwrap();
        let (accepted_tx, mut accepted_rx) = tokio::sync::mpsc::channel(2);
        let supervisor = {
            let listener = listener.clone();
            tokio::spawn(async move {
                listener
                    .serve_concurrent(move |connection, mut reader| {
                        let accepted_tx = accepted_tx.clone();
                        async move {
                            let _ = accepted_tx.send(connection.peer_addr()).await;
                            while let Some(frame) = reader.next().await {
                                let Ok(frame) = frame else {
                                    break;
                                };
                                let peer_closed = matches!(frame, WsMessage::Close(_));
                                if connection.process_ws_message(frame).is_err() || peer_closed {
                                    break;
                                }
                            }
                        }
                    })
                    .await
            })
        };
        let request = || {
            let mut request = format!("ws://{destination}/")
                .into_client_request()
                .unwrap();
            request.headers_mut().insert(
                "Sec-WebSocket-Protocol",
                http::HeaderValue::from_static("sip"),
            );
            request
        };

        let (mut first_client, _) = tokio_tungstenite::connect_async(request()).await.unwrap();
        accepted_rx.recv().await.expect("first handler not started");
        first_client.send(WsMessage::Close(None)).await.unwrap();
        assert!(matches!(
            tokio::time::timeout(std::time::Duration::from_millis(250), first_client.next())
                .await
                .expect("server did not acknowledge peer Close promptly"),
            Some(Ok(WsMessage::Close(_))) | None
        ));
        tokio::task::yield_now().await;

        let (second_client, _) = tokio::time::timeout(
            std::time::Duration::from_millis(500),
            tokio_tungstenite::connect_async(request()),
        )
        .await
        .expect("second handshake waited for the full first-session write timeout")
        .unwrap();
        tokio::time::timeout(std::time::Duration::from_millis(500), accepted_rx.recv())
            .await
            .expect("peer Close did not release established-session capacity")
            .expect("supervisor stopped before the replacement session");

        drop(second_client);
        supervisor.abort();
        let _ = supervisor.await;
    }

    /// Test binding a secure WebSocket listener.
    ///
    /// Phase 4 wired real TLS into `bind()`: when `secure = true`, the
    /// listener now actually opens the cert/key files and builds a
    /// `TlsAcceptor`. So the test has to generate real PEM material
    /// (via `rcgen`, the same dev-dep the TLS handshake test uses).
    /// Gated on `wss` because the TLS acceptor lives behind that
    /// feature.
    #[cfg(feature = "wss")]
    #[tokio::test]
    async fn test_websocket_listener_bind_secure() {
        use std::io::Write;
        let tmp = tempfile::tempdir().expect("tempdir");
        let cert_path = tmp.path().join("server.crt");
        let key_path = tmp.path().join("server.key");

        let cert = rcgen::generate_simple_self_signed(vec!["localhost".to_string()])
            .expect("rcgen self-signed");
        std::fs::File::create(&cert_path)
            .and_then(|mut f| f.write_all(cert.cert.pem().as_bytes()))
            .expect("write cert");
        std::fs::File::create(&key_path)
            .and_then(|mut f| f.write_all(cert.signing_key.serialize_pem().as_bytes()))
            .expect("write key");

        let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let listener = WebSocketListener::bind(
            addr,
            true,
            Some(cert_path.to_str().unwrap()),
            Some(key_path.to_str().unwrap()),
        )
        .await
        .unwrap();

        let bound_addr = listener.local_addr().unwrap();
        assert!(bound_addr.port() > 0); // Random port assigned
        assert_eq!(bound_addr.ip(), addr.ip());
        assert!(listener.is_secure());

        // Verify certificate paths are stored
        assert_eq!(listener.cert_path.as_deref(), cert_path.to_str());
        assert_eq!(listener.key_path.as_deref(), key_path.to_str());
    }
}
