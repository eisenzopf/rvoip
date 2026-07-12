mod connection;
mod listener;
mod stream;

pub use connection::WebSocketConnection;
pub use listener::WebSocketListener;
pub(crate) use stream::SipWsStream;

use crate::error::{Error, Result};
use crate::transport::{validate_typed_outbound_message, Transport, TransportEvent, TransportType};
use futures_util::StreamExt;
use rvoip_sip_core::Message;
use std::collections::HashMap;
use std::fmt;
use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
#[cfg(feature = "ws")]
use tokio_tungstenite::tungstenite;
use tracing::{debug, error, info, warn};

#[cfg(feature = "wss")]
pub use crate::transport::tls::{TlsClientConfig, TlsServerClientAuthConfig};
#[cfg(feature = "wss")]
use tokio_rustls::TlsConnector;

// SIP WebSocket subprotocol names as per RFC 7118
pub(crate) const SIP_WS_SUBPROTOCOL: &str = "sip";
pub(crate) const SIP_WSS_SUBPROTOCOL: &str = "sips";

// Default channel capacity
const DEFAULT_CHANNEL_CAPACITY: usize = 1000;

/// WebSocket transport for SIP messages
#[derive(Clone)]
pub struct WebSocketTransport {
    inner: Arc<WebSocketTransportInner>,
}

struct WebSocketTransportInner {
    listener: Arc<WebSocketListener>,
    secure: bool,
    connections: Mutex<HashMap<SocketAddr, Arc<WebSocketConnection>>>,
    closed: AtomicBool,
    events_tx: mpsc::Sender<TransportEvent>,
    /// `TlsConnector` used by outbound `wss://` dials. `None` when
    /// `secure=false` or when no `TlsClientConfig` was supplied at
    /// bind time — `connect_to()` then errors with `NotImplemented`
    /// for `wss://` (matches pre-Phase-4-polish behaviour).
    #[cfg(feature = "wss")]
    tls_connector: Option<TlsConnector>,
}

impl WebSocketTransport {
    /// Creates a new WebSocket transport bound to the specified address.
    ///
    /// Equivalent to [`Self::bind_with_client_tls`] with `client_tls = None`
    /// — outbound `wss://` dials remain `NotImplemented` until a
    /// `TlsClientConfig` is supplied.
    pub async fn bind(
        addr: SocketAddr,
        secure: bool,
        cert_path: Option<&str>,
        key_path: Option<&str>,
        channel_capacity: Option<usize>,
    ) -> Result<(Self, mpsc::Receiver<TransportEvent>)> {
        #[cfg(feature = "wss")]
        {
            Self::bind_with_client_tls(addr, secure, cert_path, key_path, channel_capacity, None)
                .await
        }
        #[cfg(not(feature = "wss"))]
        {
            Self::bind_inner(addr, secure, cert_path, key_path, channel_capacity).await
        }
    }

    /// Creates a WebSocket transport with an optional outbound TLS
    /// client configuration. When `secure = true` and `client_tls` is
    /// `Some`, outbound `wss://` dials run a rustls handshake using
    /// the supplied root-store / verifier policy before the WS upgrade.
    /// When `client_tls` is `None`, outbound `wss://` still returns
    /// `NotImplemented` for backwards compatibility with callers that
    /// only need server-side WSS.
    #[cfg(feature = "wss")]
    pub async fn bind_with_client_tls(
        addr: SocketAddr,
        secure: bool,
        cert_path: Option<&str>,
        key_path: Option<&str>,
        channel_capacity: Option<usize>,
        client_tls: Option<TlsClientConfig>,
    ) -> Result<(Self, mpsc::Receiver<TransportEvent>)> {
        Self::bind_with_tls_configs(
            addr,
            secure,
            cert_path,
            key_path,
            channel_capacity,
            client_tls,
            TlsServerClientAuthConfig::default(),
        )
        .await
    }

    /// Creates a WebSocket transport with independent outbound WSS client
    /// configuration and inbound WSS client-certificate authentication.
    #[cfg(feature = "wss")]
    pub async fn bind_with_tls_configs(
        addr: SocketAddr,
        secure: bool,
        cert_path: Option<&str>,
        key_path: Option<&str>,
        channel_capacity: Option<usize>,
        client_tls: Option<TlsClientConfig>,
        server_client_auth: TlsServerClientAuthConfig,
    ) -> Result<(Self, mpsc::Receiver<TransportEvent>)> {
        let tls_connector = match (secure, client_tls) {
            (true, Some(cfg)) => {
                let client_config = crate::transport::tls::build_client_config(&cfg)?;
                Some(TlsConnector::from(Arc::new(client_config)))
            }
            _ => None,
        };
        Self::bind_inner_with_connector(
            addr,
            secure,
            cert_path,
            key_path,
            channel_capacity,
            tls_connector,
            server_client_auth,
        )
        .await
    }

    /// Internal bind path shared by [`Self::bind`] and
    /// [`Self::bind_with_client_tls`]. Lives here so the non-WSS build
    /// can use a slimmer signature without referencing `TlsConnector`.
    #[cfg(feature = "wss")]
    async fn bind_inner_with_connector(
        addr: SocketAddr,
        secure: bool,
        cert_path: Option<&str>,
        key_path: Option<&str>,
        channel_capacity: Option<usize>,
        tls_connector: Option<TlsConnector>,
        server_client_auth: TlsServerClientAuthConfig,
    ) -> Result<(Self, mpsc::Receiver<TransportEvent>)> {
        // Create the event channel
        let capacity = channel_capacity.unwrap_or(DEFAULT_CHANNEL_CAPACITY);
        let (events_tx, events_rx) = mpsc::channel(capacity);

        // Create the WebSocket listener
        let listener = WebSocketListener::bind_with_client_auth(
            addr,
            secure,
            cert_path,
            key_path,
            server_client_auth,
        )
        .await?;
        let local_addr = listener.local_addr()?;

        info!(
            "SIP WebSocket transport bound to {} ({}) [client_tls: {}]",
            local_addr,
            if secure { "wss" } else { "ws" },
            if tls_connector.is_some() {
                "configured"
            } else {
                "none"
            }
        );

        let transport = WebSocketTransport {
            inner: Arc::new(WebSocketTransportInner {
                listener: Arc::new(listener),
                secure,
                connections: Mutex::new(HashMap::new()),
                closed: AtomicBool::new(false),
                events_tx: events_tx.clone(),
                tls_connector,
            }),
        };

        #[cfg(feature = "ws")]
        transport.spawn_accept_loop();

        Ok((transport, events_rx))
    }

    /// Non-WSS bind path — kept structurally identical so the
    /// `#[cfg]` branches in `bind()` don't drift.
    #[cfg(not(feature = "wss"))]
    async fn bind_inner(
        addr: SocketAddr,
        secure: bool,
        cert_path: Option<&str>,
        key_path: Option<&str>,
        channel_capacity: Option<usize>,
    ) -> Result<(Self, mpsc::Receiver<TransportEvent>)> {
        let capacity = channel_capacity.unwrap_or(DEFAULT_CHANNEL_CAPACITY);
        let (events_tx, events_rx) = mpsc::channel(capacity);

        let listener = WebSocketListener::bind(addr, secure, cert_path, key_path).await?;
        let local_addr = listener.local_addr()?;

        info!(
            "SIP WebSocket transport bound to {} ({})",
            local_addr,
            if secure { "wss" } else { "ws" }
        );

        let transport = WebSocketTransport {
            inner: Arc::new(WebSocketTransportInner {
                listener: Arc::new(listener),
                secure,
                connections: Mutex::new(HashMap::new()),
                closed: AtomicBool::new(false),
                events_tx: events_tx.clone(),
            }),
        };

        #[cfg(feature = "ws")]
        transport.spawn_accept_loop();

        Ok((transport, events_rx))
    }

    /// Spawns a task to accept incoming connections
    #[cfg(feature = "ws")]
    fn spawn_accept_loop(&self) {
        let transport = self.clone();

        tokio::spawn(async move {
            let inner = &transport.inner;
            let listener_clone = inner.listener.clone();

            while !inner.closed.load(Ordering::Relaxed) {
                // Accept a new connection
                match listener_clone.accept().await {
                    Ok((connection, reader)) => {
                        let peer_addr = connection.peer_addr();
                        debug!("Accepted WebSocket connection from {}", peer_addr);

                        // Store the connection
                        let connection_arc = Arc::new(connection);
                        {
                            let mut connections = inner.connections.lock().await;
                            connections.insert(peer_addr, connection_arc.clone());
                        }

                        // Handle the connection
                        transport
                            .clone()
                            .spawn_connection_reader(connection_arc, reader);
                    }
                    Err(e) => {
                        if inner.closed.load(Ordering::Relaxed) {
                            break;
                        }

                        error!("Error accepting WebSocket connection: {}", e);
                        let _ = inner
                            .events_tx
                            .send(TransportEvent::Error {
                                error: format!("Accept error: {}", e),
                            })
                            .await;

                        // Brief pause to avoid tight accept loop on errors
                        tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                    }
                }
            }

            // Notify that the transport is closed
            info!("WebSocket accept loop terminated");
            let _ = inner.events_tx.send(TransportEvent::Closed).await;
        });
    }

    /// Spawns a task to read messages from a connection
    #[cfg(feature = "ws")]
    fn spawn_connection_reader(
        &self,
        connection: Arc<WebSocketConnection>,
        mut reader: futures_util::stream::SplitStream<
            tokio_tungstenite::WebSocketStream<SipWsStream>,
        >,
    ) {
        let transport = self.clone();
        let peer_addr = connection.peer_addr();

        tokio::spawn(async move {
            let inner = &transport.inner;

            while !inner.closed.load(Ordering::Relaxed) && !connection.is_closed() {
                // Read the next WebSocket message
                let ws_message = match reader.next().await {
                    Some(Ok(msg)) => msg,
                    Some(Err(e)) => {
                        // Distinguish "peer disconnected" from a real
                        // protocol fault. RFC 6455 §5.5.1 says peers
                        // SHOULD send a Close frame, but in practice
                        // browsers, mobile networks, and load
                        // balancers routinely just drop the socket.
                        // tokio-tungstenite surfaces those as
                        // `ConnectionClosed`, `AlreadyClosed`, or an
                        // I/O error with `UnexpectedEof` /
                        // `ConnectionReset` / `BrokenPipe`. None of
                        // those should fire `TransportEvent::Error` or
                        // log at ERROR — they're the normal disconnect
                        // path. Anything else (`Protocol`, `Utf8`,
                        // bad frame, etc.) is a real fault.
                        let is_normal_close = match &e {
                            tungstenite::Error::ConnectionClosed
                            | tungstenite::Error::AlreadyClosed => true,
                            tungstenite::Error::Io(io_err) => matches!(
                                io_err.kind(),
                                io::ErrorKind::UnexpectedEof
                                    | io::ErrorKind::ConnectionReset
                                    | io::ErrorKind::BrokenPipe
                            ),
                            _ => false,
                        };

                        if is_normal_close {
                            debug!(
                                "WebSocket connection from {} closed by peer: {}",
                                peer_addr, e
                            );
                        } else {
                            error!(
                                "Error reading from WebSocket connection {}: {}",
                                peer_addr, e
                            );
                            let _ = inner
                                .events_tx
                                .send(TransportEvent::Error {
                                    error: format!(
                                        "WebSocket read error from {}: {}",
                                        peer_addr, e
                                    ),
                                })
                                .await;
                        }

                        break;
                    }
                    None => {
                        // End of stream
                        debug!("WebSocket connection from {} closed by peer", peer_addr);
                        break;
                    }
                };

                // Process the WebSocket message
                match connection.process_ws_message(ws_message) {
                    Ok(Some((sip_message, raw_bytes))) => {
                        debug!("Received SIP message from {}", peer_addr);

                        // Get local address (for consistency with other transports)
                        let local_addr = match inner.listener.local_addr() {
                            Ok(addr) => addr,
                            Err(e) => {
                                error!("Failed to get local address: {}", e);
                                continue;
                            }
                        };

                        // Send the event
                        let event = TransportEvent::MessageReceived {
                            message: sip_message,
                            source: peer_addr,
                            destination: local_addr,
                            transport_type: if inner.secure {
                                TransportType::Wss
                            } else {
                                TransportType::Ws
                            },
                            raw_bytes: Some(raw_bytes),
                            timing: None,
                            connection_metadata: connection.connection_metadata().cloned(),
                        };

                        if let Err(e) = inner.events_tx.send(event).await {
                            error!("Error sending event: {}", e);
                            break;
                        }
                    }
                    Ok(None) => {
                        // Control message like ping/pong/close, already handled
                        continue;
                    }
                    Err(e) => {
                        warn!(
                            "Error processing WebSocket message from {}: {}",
                            peer_addr, e
                        );

                        let _ = inner
                            .events_tx
                            .send(TransportEvent::Error {
                                error: format!("WebSocket message processing error: {}", e),
                            })
                            .await;
                    }
                }
            }

            // Connection closed, remove it from the map
            {
                let mut connections = inner.connections.lock().await;
                connections.remove(&peer_addr);
            }

            // Ensure the connection is closed
            if !connection.is_closed() {
                if let Err(e) = connection.close().await {
                    error!("Error closing WebSocket connection to {}: {}", peer_addr, e);
                }
            }

            debug!("WebSocket connection reader for {} terminated", peer_addr);
        });
    }

    /// Connect to a remote WebSocket server.
    ///
    /// Implements RFC 7118 §4.5 client-side WebSocket establishment:
    ///
    /// 1. Open a TCP connection to `addr`.
    /// 2. For WSS, wrap the TCP stream with a `tokio_rustls`
    ///    `TlsConnector` (built at bind time from the supplied
    ///    [`TlsClientConfig`]). `bind()` without a client TLS config
    ///    leaves the connector unset and WSS dials error with
    ///    `NotImplemented`; use [`Self::bind_with_client_tls`].
    /// 3. Build a WS handshake request with
    ///    `Sec-WebSocket-Protocol: sip` (or `sips` for WSS) per
    ///    RFC 7118 §4.5.
    /// 4. Call `tokio_tungstenite::client_async` to negotiate the
    ///    WS upgrade on the established stream (plain TCP or TLS).
    /// 5. Register the resulting connection in the pool and spawn
    ///    its reader so inbound messages from the server reach
    ///    `TransportEvent::MessageReceived`.
    ///
    /// Idempotent: a second call for the same `addr` returns the
    /// existing connection if it's still open.
    ///
    /// `server_name_hint` is the SNI override for the WSS handshake.
    /// When `None`, falls back to `ip_to_server_name(addr)` (loopback
    /// → `"localhost"`, otherwise an IP-typed `ServerName`). Callers
    /// with a known DNS hostname (the URI's host) should pass it
    /// through so production CA-validated WSS handshakes resolve
    /// correctly. The plain-WS path ignores this argument.
    #[cfg(feature = "ws")]
    async fn connect_to(
        &self,
        addr: SocketAddr,
        #[cfg(feature = "wss")] server_name_hint: Option<
            tokio_rustls::rustls::pki_types::ServerName<'static>,
        >,
        #[cfg(not(feature = "wss"))] _server_name_hint: (),
    ) -> Result<Arc<WebSocketConnection>> {
        // Check if we already have an open connection
        {
            let connections = self.inner.connections.lock().await;
            if let Some(conn) = connections.get(&addr) {
                if !conn.is_closed() {
                    return Ok(conn.clone());
                }
            }
        }

        use tokio_tungstenite::tungstenite::client::IntoClientRequest;

        // Pre-flight: for WSS dials, the TlsConnector must have been
        // configured at bind time (via `bind_with_client_tls`).
        // Surface this BEFORE opening TCP so the failure mode is
        // obvious and doesn't depend on whether the destination is
        // listening.
        #[cfg(feature = "wss")]
        if self.inner.secure && self.inner.tls_connector.is_none() {
            return Err(Error::NotImplemented(
                "WSS client requires TlsClientConfig — use \
                 WebSocketTransport::bind_with_client_tls instead of bind()"
                    .into(),
            ));
        }

        // Step 1 — open TCP. The destination IP/port were resolved
        // by the upper layer; we don't do DNS here.
        let tcp_stream = tokio::net::TcpStream::connect(addr)
            .await
            .map_err(|e| Error::ConnectFailed(addr, e))?;

        // Step 2 — when `secure=true`, run the rustls handshake on
        // the TCP stream BEFORE the WS upgrade (RFC 7118 §3 — wss is
        // WS-over-TLS). The connector was built once at bind time
        // from the supplied `TlsClientConfig`.
        let (stream, subprotocol_advertised, url_scheme): (
            SipWsStream,
            &'static str,
            &'static str,
        ) = if self.inner.secure {
            #[cfg(feature = "wss")]
            {
                let connector = self
                    .inner
                    .tls_connector
                    .as_ref()
                    .expect("pre-flight guarantees tls_connector is Some when secure");
                let server_name = server_name_hint
                    .unwrap_or_else(|| crate::transport::tls::ip_to_server_name(addr));
                let tls_stream = connector
                    .connect(server_name, tcp_stream)
                    .await
                    .map_err(|e| {
                        Error::TlsHandshakeFailed(format!(
                            "WSS client handshake with {}: {}",
                            addr, e
                        ))
                    })?;
                (
                    SipWsStream::ClientTls(tls_stream),
                    SIP_WSS_SUBPROTOCOL,
                    "wss",
                )
            }
            #[cfg(not(feature = "wss"))]
            {
                return Err(Error::NotImplemented(
                    "WSS client requires the `wss` cargo feature (rustls plumbing)".into(),
                ));
            }
        } else {
            (SipWsStream::Plain(tcp_stream), SIP_WS_SUBPROTOCOL, "ws")
        };

        // Step 3 — build the WS handshake URL + subprotocol header.
        // Per RFC 7118 §4.5 the client advertises `sip` for ws:// and
        // `sips` for wss://.
        let url = format!("{}://{}/", url_scheme, addr);
        let mut request = url
            .into_client_request()
            .map_err(|e| Error::WebSocketHandshakeFailed(e.to_string()))?;
        request.headers_mut().insert(
            "Sec-WebSocket-Protocol",
            http::HeaderValue::from_static(subprotocol_advertised),
        );

        // Step 4 — run the WS upgrade on whichever stream variant we
        // ended up with (Plain or ClientTls — they both implement
        // AsyncRead+AsyncWrite via SipWsStream).
        let (ws_stream, response) = tokio_tungstenite::client_async(request, stream)
            .await
            .map_err(|e| Error::WebSocketHandshakeFailed(e.to_string()))?;

        // Capture the server's selected subprotocol so the connection
        // wrapper carries the negotiated value (mirrors what the
        // listener path does).
        let selected_subprotocol = response
            .headers()
            .get("Sec-WebSocket-Protocol")
            .and_then(|v| v.to_str().ok())
            .map(str::to_string)
            .unwrap_or_else(|| subprotocol_advertised.to_string());

        let (ws_writer, ws_reader) = ws_stream.split();

        let connection = WebSocketConnection::from_writer(
            ws_writer,
            addr,
            self.inner.secure,
            selected_subprotocol,
        );
        let connection_arc = Arc::new(connection);

        // Register in the pool so subsequent send_message calls
        // reuse the same connection.
        {
            let mut connections = self.inner.connections.lock().await;
            connections.insert(addr, connection_arc.clone());
        }

        // Spawn the reader so server-pushed responses (typical SIP
        // case — UAS replies on the same WS the UAC opened) reach
        // TransportEvent::MessageReceived.
        self.clone()
            .spawn_connection_reader(connection_arc.clone(), ws_reader);

        debug!(
            "WebSocket client connected to {} (subprotocol={})",
            addr,
            connection_arc.subprotocol()
        );

        Ok(connection_arc)
    }
}

#[async_trait::async_trait]
impl Transport for WebSocketTransport {
    fn local_addr(&self) -> Result<SocketAddr> {
        self.inner.listener.local_addr()
    }

    async fn send_message(&self, message: Message, destination: SocketAddr) -> Result<()> {
        if self.is_closed() {
            return Err(Error::TransportClosed);
        }
        validate_typed_outbound_message(&message)?;

        debug!(
            "Sending {} message to {}",
            if let Message::Request(ref req) = message {
                format!("{}", req.method)
            } else {
                "response".to_string()
            },
            destination
        );

        #[cfg(feature = "ws")]
        {
            // For WSS, derive SNI from the request's next-hop URI
            // host so production CA-validated handshakes resolve.
            // Mirrors the TLS transport's `tls_server_name_for_message`
            // pattern. Plain WS ignores this.
            #[cfg(feature = "wss")]
            let server_name = if self.inner.secure {
                crate::transport::tls::tls_server_name_for_message(&message, destination)
            } else {
                None
            };
            #[cfg(not(feature = "wss"))]
            let server_name = ();

            let connection = self.connect_to(destination, server_name).await?;

            // Send the message
            connection.send_message(&message).await
        }

        #[cfg(not(feature = "ws"))]
        Err(Error::NotImplemented(
            "WebSocket transport not implemented".into(),
        ))
    }

    async fn send_message_raw(&self, bytes: bytes::Bytes, destination: SocketAddr) -> Result<()> {
        if self.is_closed() {
            return Err(Error::TransportClosed);
        }
        debug!(
            "WS: sending {} pre-built bytes to {}",
            bytes.len(),
            destination
        );

        #[cfg(feature = "ws")]
        {
            // Raw-bytes send doesn't have a parsed message to derive
            // SNI from. Fall back to IP-derived ServerName.
            #[cfg(feature = "wss")]
            let server_name: Option<
                tokio_rustls::rustls::pki_types::ServerName<'static>,
            > = None;
            #[cfg(not(feature = "wss"))]
            let server_name = ();

            let connection = self.connect_to(destination, server_name).await?;
            connection.send_raw_bytes(bytes).await
        }

        #[cfg(not(feature = "ws"))]
        Err(Error::NotImplemented(
            "WebSocket transport not implemented".into(),
        ))
    }

    async fn close(&self) -> Result<()> {
        // Set the closed flag to prevent new operations
        self.inner.closed.store(true, Ordering::Relaxed);

        // Close all connections
        let mut connections = self.inner.connections.lock().await;
        for (addr, conn) in connections.drain() {
            if let Err(e) = conn.close().await {
                error!("Error closing WebSocket connection to {}: {}", addr, e);
            }
        }

        Ok(())
    }

    fn is_closed(&self) -> bool {
        self.inner.closed.load(Ordering::Relaxed)
    }
}

impl fmt::Debug for WebSocketTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self.inner.listener.local_addr() {
            Ok(addr) => write!(f, "WebSocketTransport({})", addr),
            Err(_) => write!(f, "WebSocketTransport(<error>)"),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rvoip_sip_core::builder::SimpleRequestBuilder;
    use rvoip_sip_core::types::headers::{HeaderName, HeaderValue, TypedHeader};
    use rvoip_sip_core::Method;
    use tokio::time::Duration;

    #[cfg(feature = "ws")]
    #[tokio::test]
    async fn test_websocket_transport_bind() {
        let result =
            WebSocketTransport::bind("127.0.0.1:0".parse().unwrap(), false, None, None, None).await;

        if cfg!(feature = "ws") {
            let (transport, _rx) = result.unwrap();
            let addr = transport.local_addr().unwrap();
            assert!(addr.port() > 0);

            transport.close().await.unwrap();
            assert!(transport.is_closed());
        } else {
            assert!(result.is_err());
        }
    }

    #[cfg(feature = "ws")]
    #[tokio::test]
    async fn typed_ws_and_wss_boundary_rejects_auth_before_connect() {
        let (transport, _rx) =
            WebSocketTransport::bind("127.0.0.1:0".parse().unwrap(), false, None, None, None)
                .await
                .unwrap();
        let destination = "127.0.0.1:9".parse().unwrap();
        let mut request = SimpleRequestBuilder::new(Method::Options, "sip:example.com")
            .unwrap()
            .build();
        request.headers.push(TypedHeader::Other(
            HeaderName::ProxyAuthorization,
            HeaderValue::Raw(b"Digest safe\r\nX-Injected: websocket".to_vec()),
        ));

        let error = transport
            .send_message(Message::Request(request), destination)
            .await
            .expect_err("typed WS/WSS send must reject credential injection");
        assert!(matches!(error, Error::ProtocolError(_)));
        assert!(!error.to_string().contains("X-Injected"));
        transport.close().await.unwrap();
    }

    /// Phase 4 wired real cert/key loading into the WSS bind path, so
    /// this test needs PEM material that actually exists on disk.
    /// Gated on `wss` because the TLS acceptor lives behind that
    /// feature.
    #[cfg(feature = "wss")]
    #[tokio::test]
    async fn test_websocket_transport_secure_bind() {
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

        let (transport, _rx) = WebSocketTransport::bind(
            "127.0.0.1:0".parse().unwrap(),
            true,
            Some(cert_path.to_str().unwrap()),
            Some(key_path.to_str().unwrap()),
            None,
        )
        .await
        .unwrap();

        let addr = transport.local_addr().unwrap();
        assert!(addr.port() > 0);

        transport.close().await.unwrap();
        assert!(transport.is_closed());
    }

    /// Phase 4 polish: WSS client is wired through
    /// `bind_with_client_tls`. Plain `bind()` callers still get
    /// `NotImplemented` for WSS dials — this test ensures that opt-in
    /// gate doesn't silently break (e.g., a future refactor that
    /// auto-builds a TlsConnector with default roots regardless of
    /// caller intent).
    #[cfg(feature = "wss")]
    #[tokio::test]
    async fn test_wss_client_without_client_tls_config_is_not_implemented() {
        use std::io::Write;

        // The listener side needs cert+key now that `secure=true`
        // actually loads them. Generate self-signed material — the test
        // never accepts a connection, just verifies the *client* path
        // bails out.
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

        let (transport, _rx) = WebSocketTransport::bind(
            "127.0.0.1:0".parse().unwrap(),
            true,
            Some(cert_path.to_str().unwrap()),
            Some(key_path.to_str().unwrap()),
            None,
        )
        .await
        .unwrap();

        let request = SimpleRequestBuilder::new(Method::Register, "sip:example.com")
            .unwrap()
            .from("alice", "sip:alice@example.com", Some("tag1"))
            .to("bob", "sip:bob@example.com", None)
            .call_id("call1@example.com")
            .cseq(1)
            .build();

        // Sending via this WSS transport routes through `connect_to`'s
        // secure arm, which currently returns NotImplemented.
        // Destination doesn't have to be live — the failure happens
        // before any TCP connect is attempted.
        let result = transport
            .send_message(request.into(), "127.0.0.1:1".parse().unwrap())
            .await;
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(
                matches!(e, Error::NotImplemented(_)),
                "expected NotImplemented for WSS client, got {:?}",
                e
            );
        }

        transport.close().await.unwrap();
    }

    #[cfg(feature = "ws")]
    #[tokio::test]
    async fn test_websocket_transport_event_channels() {
        // Test that the transport correctly sets up event channels
        let channel_capacity = 42;
        let (transport, mut rx) = WebSocketTransport::bind(
            "127.0.0.1:0".parse().unwrap(),
            false,
            None,
            None,
            Some(channel_capacity),
        )
        .await
        .unwrap();

        // Close the transport - this should send a Closed event
        transport.close().await.unwrap();

        // Wait for the closed event
        let event = tokio::time::timeout(Duration::from_secs(1), rx.recv())
            .await
            .unwrap();

        // Verify the event
        assert!(matches!(event, Some(TransportEvent::Closed)));
    }

    #[cfg(feature = "ws")]
    #[tokio::test]
    async fn test_websocket_transport_debug_fmt() {
        // Test the Debug implementation
        let (transport, _rx) =
            WebSocketTransport::bind("127.0.0.1:0".parse().unwrap(), false, None, None, None)
                .await
                .unwrap();

        let debug_str = format!("{:?}", transport);
        assert!(debug_str.starts_with("WebSocketTransport(127.0.0.1:"));

        transport.close().await.unwrap();
    }

    // Tests for client connection support would go here once implemented
}
