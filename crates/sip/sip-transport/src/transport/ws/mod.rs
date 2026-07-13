mod connection;
mod listener;
mod stream;

pub use connection::WebSocketConnection;
pub use listener::WebSocketListener;
pub(crate) use stream::SipWsStream;

use crate::error::{Error, Result};
use crate::transport::{
    runtime::{
        next_trust_context, ConnectionDirection, ConnectionLifecycleConfig, DialAdmission,
        OutboundDialCoordinator, TransportTaskSet,
    },
    safe_method_label, validate_typed_outbound_message, HandshakeAdmissionConfig, Transport,
    TransportEvent, TransportType,
};
use futures_util::StreamExt;
use rvoip_sip_core::types::uri::Host;
use rvoip_sip_core::Message;
use std::collections::HashMap;
use std::fmt;
use std::io;
use std::net::{IpAddr, SocketAddr};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Weak};
use tokio::sync::{mpsc, Mutex, OwnedSemaphorePermit, Semaphore};
#[cfg(feature = "ws")]
use tokio_tungstenite::tungstenite;
use tracing::{debug, error, info, warn};

#[cfg(feature = "wss")]
pub use crate::transport::tls::{TlsClientConfig, TlsServerClientAuthConfig};
#[cfg(feature = "wss")]
use tokio_rustls::TlsConnector;

// RFC 7118 registers exactly one WebSocket subprotocol token for SIP.
// Transport security is selected by the ws:// versus wss:// URI, not by a
// second `sips` subprotocol token.
pub(crate) const SIP_WS_SUBPROTOCOL: &str = "sip";

fn selected_subprotocol_is_exact(selected: Option<&str>, expected: &str) -> bool {
    selected.is_some_and(|value| value == expected)
}

// Default channel capacity
const DEFAULT_CHANNEL_CAPACITY: usize = 1000;

/// WebSocket transport for SIP messages
#[derive(Clone)]
pub struct WebSocketTransport {
    inner: Arc<WebSocketTransportInner>,
}

struct WebSocketTransportInner {
    local_addr: SocketAddr,
    secure: bool,
    connections: Mutex<HashMap<WebSocketConnectionKey, WebSocketConnectionRecord>>,
    next_connection_generation: AtomicU64,
    closed: AtomicBool,
    close_gate: Mutex<()>,
    events_tx: mpsc::Sender<TransportEvent>,
    tasks: Arc<TransportTaskSet>,
    handshake_admission: HandshakeAdmissionConfig,
    lifecycle: ConnectionLifecycleConfig,
    outbound_dials: Arc<OutboundDialCoordinator<WebSocketConnectionKey>>,
    inbound_established: Arc<Semaphore>,
    outbound_established: Arc<Semaphore>,
    inbound_trust_context: u64,
    outbound_trust_context: u64,
    /// `TlsConnector` used by outbound `wss://` dials. `None` when
    /// `secure=false` or when no `TlsClientConfig` was supplied at
    /// bind time — `connect_to()` then errors with `NotImplemented`
    /// for `wss://` (matches pre-Phase-4-polish behaviour).
    #[cfg(feature = "wss")]
    tls_connector: Option<TlsConnector>,
}

#[derive(Clone)]
struct WebSocketConnectionRecord {
    generation: u64,
    connection: Arc<WebSocketConnection>,
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
enum WebSocketAuthorityHost {
    Dns(String),
    Ip(IpAddr),
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct WebSocketAuthority {
    host: WebSocketAuthorityHost,
    port: u16,
}

impl WebSocketAuthority {
    fn for_message(message: &Message, destination: SocketAddr) -> Self {
        let host = match message {
            Message::Request(request) => match &request.uri().host {
                Host::Domain(domain) => {
                    WebSocketAuthorityHost::Dns(domain.trim_end_matches('.').to_ascii_lowercase())
                }
                Host::Address(address) => WebSocketAuthorityHost::Ip(*address),
            },
            Message::Response(_) => return Self::for_address(destination),
        };
        Self {
            host,
            port: destination.port(),
        }
    }

    fn for_address(destination: SocketAddr) -> Self {
        let host = if destination.ip().is_loopback() {
            WebSocketAuthorityHost::Dns("localhost".into())
        } else {
            WebSocketAuthorityHost::Ip(destination.ip())
        };
        Self {
            host,
            port: destination.port(),
        }
    }

    fn http_authority(&self) -> String {
        match self.host {
            WebSocketAuthorityHost::Dns(ref domain) => format!("{domain}:{}", self.port),
            WebSocketAuthorityHost::Ip(IpAddr::V4(address)) => {
                format!("{address}:{}", self.port)
            }
            WebSocketAuthorityHost::Ip(IpAddr::V6(address)) => {
                format!("[{address}]:{}", self.port)
            }
        }
    }

    #[cfg(feature = "wss")]
    fn server_name(&self) -> Result<tokio_rustls::rustls::pki_types::ServerName<'static>> {
        match &self.host {
            WebSocketAuthorityHost::Dns(domain) => {
                tokio_rustls::rustls::pki_types::ServerName::try_from(domain.clone())
                    .map_err(|_| Error::InvalidAddress("invalid WSS authority".into()))
            }
            WebSocketAuthorityHost::Ip(address) => Ok((*address).into()),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Hash)]
struct WebSocketConnectionKey {
    remote_addr: SocketAddr,
    direction: ConnectionDirection,
    authority: WebSocketAuthority,
    trust_context: u64,
    secure: bool,
}

impl WebSocketConnectionKey {
    fn outbound(
        remote_addr: SocketAddr,
        authority: WebSocketAuthority,
        trust_context: u64,
        secure: bool,
    ) -> Self {
        Self {
            remote_addr,
            direction: ConnectionDirection::Outbound,
            authority,
            trust_context,
            secure,
        }
    }

    fn inbound(
        remote_addr: SocketAddr,
        local_addr: SocketAddr,
        trust_context: u64,
        secure: bool,
    ) -> Self {
        Self {
            remote_addr,
            direction: ConnectionDirection::Inbound,
            authority: WebSocketAuthority::for_address(local_addr),
            trust_context,
            secure,
        }
    }
}

fn select_unambiguous_websocket_flow(
    connections: &HashMap<WebSocketConnectionKey, WebSocketConnectionRecord>,
    destination: SocketAddr,
) -> Result<Option<Arc<WebSocketConnection>>> {
    if let Some(inbound) = connections.iter().find_map(|(key, record)| {
        (key.remote_addr == destination
            && key.direction == ConnectionDirection::Inbound
            && !record.connection.is_closed())
        .then(|| record.connection.clone())
    }) {
        return Ok(Some(inbound));
    }

    let mut outbound = connections.iter().filter(|(key, record)| {
        key.remote_addr == destination
            && key.direction == ConnectionDirection::Outbound
            && !record.connection.is_closed()
    });
    let first = outbound.next().map(|(_, record)| record.connection.clone());
    if outbound.next().is_some() {
        return Err(Error::InvalidState(format!(
            "Multiple authenticated WebSocket flows exist for {destination}; an explicit authority is required"
        )));
    }
    Ok(first)
}

async fn close_websocket_connections(
    connections: Vec<(WebSocketConnectionKey, WebSocketConnectionRecord)>,
) {
    // Start every bounded close at once. Drain duration is therefore one
    // writer deadline, not connection_count × writer deadline.
    let close_results =
        futures_util::future::join_all(connections.iter().map(|(key, record)| async move {
            (key.remote_addr, record.connection.close().await)
        }))
        .await;
    for (remote_addr, result) in close_results {
        if let Err(error) = result {
            error!(
                "Error closing WebSocket connection to {}: {}",
                remote_addr, error
            );
        }
    }
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
        Self::bind_with_handshake_config(
            addr,
            secure,
            cert_path,
            key_path,
            channel_capacity,
            HandshakeAdmissionConfig::default(),
        )
        .await
    }

    /// Bind with an explicit deadline and concurrency limit for inbound and
    /// outbound TCP, TLS, and HTTP/WebSocket handshakes. Each direction has an
    /// independent global budget; outbound destinations are single-flight.
    pub async fn bind_with_handshake_config(
        addr: SocketAddr,
        secure: bool,
        cert_path: Option<&str>,
        key_path: Option<&str>,
        channel_capacity: Option<usize>,
        handshake_admission: HandshakeAdmissionConfig,
    ) -> Result<(Self, mpsc::Receiver<TransportEvent>)> {
        #[cfg(feature = "wss")]
        {
            Self::bind_with_tls_configs_and_handshake(
                addr,
                secure,
                cert_path,
                key_path,
                channel_capacity,
                None,
                TlsServerClientAuthConfig::default(),
                handshake_admission,
            )
            .await
        }
        #[cfg(not(feature = "wss"))]
        {
            Self::bind_inner(
                addr,
                secure,
                cert_path,
                key_path,
                channel_capacity,
                handshake_admission,
            )
            .await
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
        Self::bind_with_tls_configs_and_handshake(
            addr,
            secure,
            cert_path,
            key_path,
            channel_capacity,
            client_tls,
            server_client_auth,
            HandshakeAdmissionConfig::default(),
        )
        .await
    }

    /// Bind with independent WSS client/server TLS policies and explicit
    /// inbound/outbound handshake admission.
    #[cfg(feature = "wss")]
    pub async fn bind_with_tls_configs_and_handshake(
        addr: SocketAddr,
        secure: bool,
        cert_path: Option<&str>,
        key_path: Option<&str>,
        channel_capacity: Option<usize>,
        client_tls: Option<TlsClientConfig>,
        server_client_auth: TlsServerClientAuthConfig,
        handshake_admission: HandshakeAdmissionConfig,
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
            handshake_admission,
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
        handshake_admission: HandshakeAdmissionConfig,
    ) -> Result<(Self, mpsc::Receiver<TransportEvent>)> {
        let handshake_admission =
            handshake_admission.validate(if secure { "WSS" } else { "WS" })?;
        let lifecycle = ConnectionLifecycleConfig::from_handshake(handshake_admission);
        // Create the event channel
        let capacity = channel_capacity.unwrap_or(DEFAULT_CHANNEL_CAPACITY);
        let (events_tx, events_rx) = mpsc::channel(capacity);

        // Create the WebSocket listener
        let listener = WebSocketListener::bind_with_client_auth_and_handshake(
            addr,
            secure,
            cert_path,
            key_path,
            server_client_auth,
            handshake_admission,
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
                local_addr,
                secure,
                connections: Mutex::new(HashMap::new()),
                next_connection_generation: AtomicU64::new(1),
                closed: AtomicBool::new(false),
                close_gate: Mutex::new(()),
                events_tx: events_tx.clone(),
                tasks: TransportTaskSet::new(),
                handshake_admission,
                lifecycle,
                outbound_dials: OutboundDialCoordinator::new(
                    handshake_admission.max_concurrent,
                    lifecycle.max_pending_dials,
                    lifecycle.failure_backoff,
                ),
                inbound_established: Arc::new(Semaphore::new(
                    lifecycle.max_established_per_direction,
                )),
                outbound_established: Arc::new(Semaphore::new(
                    lifecycle.max_established_per_direction,
                )),
                inbound_trust_context: next_trust_context(),
                outbound_trust_context: next_trust_context(),
                tls_connector,
            }),
        };

        #[cfg(feature = "ws")]
        transport.spawn_accept_loop(Arc::new(listener)).await?;

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
        handshake_admission: HandshakeAdmissionConfig,
    ) -> Result<(Self, mpsc::Receiver<TransportEvent>)> {
        let handshake_admission =
            handshake_admission.validate(if secure { "WSS" } else { "WS" })?;
        let lifecycle = ConnectionLifecycleConfig::from_handshake(handshake_admission);
        let capacity = channel_capacity.unwrap_or(DEFAULT_CHANNEL_CAPACITY);
        let (events_tx, events_rx) = mpsc::channel(capacity);

        let listener = WebSocketListener::bind_with_handshake_config(
            addr,
            secure,
            cert_path,
            key_path,
            handshake_admission,
        )
        .await?;
        let local_addr = listener.local_addr()?;

        info!(
            "SIP WebSocket transport bound to {} ({})",
            local_addr,
            if secure { "wss" } else { "ws" }
        );

        let transport = WebSocketTransport {
            inner: Arc::new(WebSocketTransportInner {
                local_addr,
                secure,
                connections: Mutex::new(HashMap::new()),
                next_connection_generation: AtomicU64::new(1),
                closed: AtomicBool::new(false),
                close_gate: Mutex::new(()),
                events_tx: events_tx.clone(),
                tasks: TransportTaskSet::new(),
                handshake_admission,
                lifecycle,
                outbound_dials: OutboundDialCoordinator::new(
                    handshake_admission.max_concurrent,
                    lifecycle.max_pending_dials,
                    lifecycle.failure_backoff,
                ),
                inbound_established: Arc::new(Semaphore::new(
                    lifecycle.max_established_per_direction,
                )),
                outbound_established: Arc::new(Semaphore::new(
                    lifecycle.max_established_per_direction,
                )),
                inbound_trust_context: next_trust_context(),
                outbound_trust_context: next_trust_context(),
            }),
        };

        #[cfg(feature = "ws")]
        transport.spawn_accept_loop(Arc::new(listener)).await?;

        Ok((transport, events_rx))
    }

    /// Start the raw TCP accept supervisor. Handshake permits are acquired
    /// before `accept`, so userspace never owns more unauthenticated sockets
    /// than the configured limit. Each accepted socket then completes its WSS
    /// TLS and HTTP upgrade concurrently under one end-to-end deadline.
    #[cfg(feature = "ws")]
    async fn spawn_accept_loop(&self, listener: Arc<WebSocketListener>) -> Result<()> {
        let weak_inner = Arc::downgrade(&self.inner);
        let weak_tasks = Arc::downgrade(&self.inner.tasks);
        let admission = self.inner.handshake_admission;
        let semaphore = Arc::new(Semaphore::new(admission.max_concurrent));

        let accepted = self
            .inner
            .tasks
            .spawn(async move {
                loop {
                    let permit = match semaphore.clone().acquire_owned().await {
                        Ok(permit) => permit,
                        Err(_) => break,
                    };
                    let Some(inner) = weak_inner.upgrade() else {
                        break;
                    };
                    if inner.closed.load(Ordering::Acquire) {
                        break;
                    }
                    drop(inner);

                    let (stream, peer_addr) = match listener.accept_tcp().await {
                        Ok(accepted) => accepted,
                        Err(error) => {
                            error!("Error accepting WebSocket TCP connection: {}", error);
                            if let Some(inner) = weak_inner.upgrade() {
                                let _ = inner.events_tx.try_send(TransportEvent::Error {
                                    error: format!("Accept error: {error}"),
                                });
                            }
                            drop(permit);
                            tokio::time::sleep(tokio::time::Duration::from_millis(100)).await;
                            continue;
                        }
                    };

                    let Some(tasks) = weak_tasks.upgrade() else {
                        break;
                    };
                    let listener = listener.clone();
                    let weak_inner_for_handshake = weak_inner.clone();
                    let _ = tasks
                        .spawn(async move {
                            let deadline = tokio::time::Instant::now() + admission.timeout;
                            let upgraded = tokio::time::timeout_at(
                                deadline,
                                listener.upgrade_tcp(stream, peer_addr),
                            )
                            .await;

                            let (connection, reader) = match upgraded {
                                Ok(Ok(upgraded)) => upgraded,
                                Ok(Err(error)) => {
                                    warn!(
                                        source = %peer_addr,
                                        error_class = "websocket_handshake_failed",
                                        "WebSocket handshake rejected"
                                    );
                                    if let Some(inner) = weak_inner_for_handshake.upgrade() {
                                        let _ = inner.events_tx.try_send(TransportEvent::Error {
                                            error: format!("WebSocket handshake failed: {error}"),
                                        });
                                    }
                                    return;
                                }
                                Err(_) => {
                                    warn!(
                                        source = %peer_addr,
                                        timeout_ms = admission.timeout.as_millis(),
                                        "WebSocket handshake timed out"
                                    );
                                    return;
                                }
                            };

                            let Some(inner) = weak_inner_for_handshake.upgrade() else {
                                let _ = connection.close().await;
                                return;
                            };
                            if inner.closed.load(Ordering::Acquire) {
                                drop(inner);
                                let _ = connection.close().await;
                                return;
                            }

                            let established_permit =
                                match inner.inbound_established.clone().try_acquire_owned() {
                                    Ok(permit) => permit,
                                    Err(_) => {
                                        drop(inner);
                                        warn!(
                                            source = %peer_addr,
                                            "WebSocket established inbound connection limit reached"
                                        );
                                        let _ = connection.close().await;
                                        return;
                                    }
                                };

                            debug!("Accepted WebSocket connection from {}", peer_addr);
                            let connection = Arc::new(connection);
                            let generation = inner
                                .next_connection_generation
                                .fetch_add(1, Ordering::Relaxed);
                            let key = WebSocketConnectionKey::inbound(
                                peer_addr,
                                inner.local_addr,
                                inner.inbound_trust_context,
                                inner.secure,
                            );
                            let lifecycle = inner.lifecycle;
                            let mut connections =
                                match tokio::time::timeout_at(deadline, inner.connections.lock())
                                    .await
                                {
                                    Ok(connections) => connections,
                                    Err(_) => {
                                        let _ = connection.close().await;
                                        warn!(
                                            source = %peer_addr,
                                            "WebSocket registration timed out"
                                        );
                                        return;
                                    }
                                };
                            connections.insert(
                                key.clone(),
                                WebSocketConnectionRecord {
                                    generation,
                                    connection: connection.clone(),
                                },
                            );
                            drop(connections);
                            drop(permit);
                            drop(inner);

                            Self::run_connection_reader(
                                weak_inner_for_handshake,
                                connection,
                                reader,
                                generation,
                                key,
                                lifecycle,
                                established_permit,
                            )
                            .await;
                        })
                        .await;
                }
                info!("WebSocket accept loop terminated");
            })
            .await;

        if accepted.is_none() {
            return Err(Error::TransportClosed);
        }
        Ok(())
    }

    /// Read one established connection without retaining a strong reference to
    /// the transport across socket waits. This avoids a task/transport ownership
    /// cycle while still making every reader joinable by `close()`.
    #[cfg(feature = "ws")]
    async fn run_connection_reader(
        weak_inner: Weak<WebSocketTransportInner>,
        connection: Arc<WebSocketConnection>,
        mut reader: futures_util::stream::SplitStream<
            tokio_tungstenite::WebSocketStream<SipWsStream>,
        >,
        generation: u64,
        key: WebSocketConnectionKey,
        lifecycle: ConnectionLifecycleConfig,
        _established_permit: OwnedSemaphorePermit,
    ) {
        let peer_addr = connection.peer_addr();
        let mut activity = connection.activity_receiver();
        let mut writer_closed = connection.writer_closed_receiver();
        let established_at = tokio::time::Instant::now();

        loop {
            let Some(inner) = weak_inner.upgrade() else {
                break;
            };
            if inner.closed.load(Ordering::Acquire)
                || connection.is_closed()
                || *writer_closed.borrow()
            {
                break;
            }
            drop(inner);

            let deadline = lifecycle.next_deadline(*activity.borrow(), established_at);
            let next_message = tokio::select! {
                message = reader.next() => Some(message),
                changed = activity.changed() => {
                    if changed.is_err() {
                        break;
                    }
                    None
                }
                _ = writer_closed.changed() => break,
                _ = tokio::time::sleep_until(deadline) => {
                    debug!(destination = %peer_addr, "WebSocket connection lifecycle deadline reached");
                    break;
                }
            };
            let Some(next_message) = next_message else {
                continue;
            };
            // Read the next WebSocket message
            let ws_message = match next_message {
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
                        if let Some(inner) = weak_inner.upgrade() {
                            let _ = inner.events_tx.try_send(TransportEvent::Error {
                                error: format!("WebSocket read error from {}: {}", peer_addr, e),
                            });
                        }
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

                    let Some(inner) = weak_inner.upgrade() else {
                        break;
                    };
                    if inner.closed.load(Ordering::Acquire) {
                        break;
                    }

                    // Send the event
                    let event = TransportEvent::MessageReceived {
                        message: sip_message,
                        source: peer_addr,
                        destination: inner.local_addr,
                        transport_type: if inner.secure {
                            TransportType::Wss
                        } else {
                            TransportType::Ws
                        },
                        raw_bytes: Some(raw_bytes),
                        timing: None,
                        connection_metadata: connection.connection_metadata().cloned(),
                    };

                    if let Err(e) = inner.events_tx.try_send(event) {
                        warn!(
                            source = %peer_addr,
                            error = %e,
                            "WebSocket event queue unavailable; closing flow instead of blocking lifecycle cleanup"
                        );
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

                    if let Some(inner) = weak_inner.upgrade() {
                        let _ = inner.events_tx.try_send(TransportEvent::Error {
                            error: format!("WebSocket message processing error: {}", e),
                        });
                    }
                }
            }
        }

        // Connection closed, remove it from the map.
        if let Some(inner) = weak_inner.upgrade() {
            let mut connections = inner.connections.lock().await;
            if connections
                .get(&key)
                .is_some_and(|record| record.generation == generation)
            {
                connections.remove(&key);
            }
        }

        if !connection.is_closed() {
            if let Err(e) = connection.close().await {
                error!("Error closing WebSocket connection to {}: {}", peer_addr, e);
            }
        }

        debug!("WebSocket connection reader for {} terminated", peer_addr);
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
    ///    `Sec-WebSocket-Protocol: sip` for both WS and WSS per RFC 7118
    ///    §4.5.
    /// 4. Call `tokio_tungstenite::client_async` to negotiate the
    ///    WS upgrade on the established stream (plain TCP or TLS).
    /// 5. Register the resulting connection in the pool and spawn
    ///    its reader so inbound messages from the server reach
    ///    `TransportEvent::MessageReceived`.
    ///
    /// Idempotent only for the complete pool identity: remote address,
    /// normalized HTTP/TLS authority, direction, security mode, and trust
    /// context. WSS derives both `Host` and SNI from that one authority, so a
    /// connection authenticated for one virtual host is never reused for a
    /// different virtual host on the same address.
    #[cfg(feature = "ws")]
    async fn connect_to(
        &self,
        addr: SocketAddr,
        authority: WebSocketAuthority,
    ) -> Result<Arc<WebSocketConnection>> {
        let deadline = tokio::time::Instant::now() + self.inner.handshake_admission.timeout;
        let key = WebSocketConnectionKey::outbound(
            addr,
            authority.clone(),
            self.inner.outbound_trust_context,
            self.inner.secure,
        );
        {
            let mut connections = tokio::time::timeout_at(deadline, self.inner.connections.lock())
                .await
                .map_err(|_| Error::ConnectionTimeout(addr))?;
            if connections
                .get(&key)
                .is_some_and(|record| !record.connection.is_closed())
            {
                return Ok(connections[&key].connection.clone());
            }
            connections.remove(&key);
        }

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

        let inner = self.inner.clone();
        let managed_tasks = self.inner.tasks.clone();
        let coordinator = self.inner.outbound_dials.clone();

        match coordinator.begin(key.clone())? {
            DialAdmission::Follower { outcome, .. } => {
                OutboundDialCoordinator::<WebSocketConnectionKey>::wait(outcome, deadline, addr)
                    .await?;
                tokio::time::timeout_at(deadline, self.inner.connections.lock())
                    .await
                    .map_err(|_| Error::ConnectionTimeout(addr))?
                    .get(&key)
                    .filter(|record| !record.connection.is_closed())
                    .map(|record| record.connection.clone())
                    .ok_or(Error::TransportClosed)
            }
            DialAdmission::Leader {
                key,
                flight,
                _pending,
                cancellation,
            } => {
                let coordinator_for_task = coordinator.clone();
                let pending_permit = _pending;
                managed_tasks
                    .run(async move {
                        let mut cancellation = cancellation;
                        let _pending_permit = pending_permit;
                        use tokio_tungstenite::tungstenite::client::IntoClientRequest;

                        let result = async {
                            let _handshake = coordinator_for_task
                                .acquire_handshake(deadline, addr)
                                .await?;
                            {
                                let mut connections = tokio::time::timeout_at(
                                    deadline,
                                    inner.connections.lock(),
                                )
                                .await
                                .map_err(|_| Error::ConnectionTimeout(addr))?;
                                if let Some(record) = connections.get(&key) {
                                    if !record.connection.is_closed() {
                                        return Ok(record.connection.clone());
                                    }
                                }
                                connections.remove(&key);
                            }
                            let established_permit = inner
                                .outbound_established
                                .clone()
                                .try_acquire_owned()
                                .map_err(|_| Error::ConnectionPoolExhausted)?;

                            let (ws_stream, selected_subprotocol) =
                                tokio::time::timeout_at(deadline, async {
                                    let tcp_stream = tokio::net::TcpStream::connect(addr)
                                        .await
                                        .map_err(|e| Error::ConnectFailed(addr, e))?;

                                    let (stream, subprotocol_advertised, url_scheme): (
                                        SipWsStream,
                                        &'static str,
                                        &'static str,
                                    ) = if inner.secure {
                                        #[cfg(feature = "wss")]
                                        {
                                            let connector = inner.tls_connector.as_ref().expect(
                                                "pre-flight guarantees configured WSS connector",
                                            );
                                            let server_name = authority.server_name()?;
                                            let tls_stream = connector
                                                .connect(server_name, tcp_stream)
                                                .await
                                                .map_err(|error| {
                                                    crate::transport::tls::classify_tls_runtime_error(
                                                        error,
                                                        format!(
                                                            "WSS client TLS handshake failed for {addr}"
                                                        ),
                                                    )
                                                })?;
                                            (
                                                SipWsStream::ClientTls(tls_stream),
                                                SIP_WS_SUBPROTOCOL,
                                                "wss",
                                            )
                                        }
                                        #[cfg(not(feature = "wss"))]
                                        {
                                            return Err(Error::NotImplemented(
                                                "WSS client requires the `wss` cargo feature"
                                                    .into(),
                                            ));
                                        }
                                    } else {
                                        (
                                            SipWsStream::Plain(tcp_stream),
                                            SIP_WS_SUBPROTOCOL,
                                            "ws",
                                        )
                                    };

                                    // HTTP Host and TLS SNI are derived from
                                    // the same normalized authority.
                                    let url = format!(
                                        "{}://{}/",
                                        url_scheme,
                                        authority.http_authority()
                                    );
                                    let mut request =
                                        url.into_client_request().map_err(|_error| {
                                            Error::WebSocketHandshakeFailed(format!(
                                                "WebSocket client request construction failed for {addr}"
                                            ))
                                        })?;
                                    request.headers_mut().insert(
                                        "Sec-WebSocket-Protocol",
                                        http::HeaderValue::from_static(subprotocol_advertised),
                                    );
                                    let (ws_stream, response) =
                                        tokio_tungstenite::client_async_with_config(
                                            request,
                                            stream,
                                            Some(connection::sip_websocket_config()),
                                        )
                                        .await
                                        .map_err(|_error| {
                                            Error::WebSocketHandshakeFailed(format!(
                                                "WebSocket client handshake failed for {addr}"
                                            ))
                                        })?;
                                    let selected = response
                                        .headers()
                                        .get("Sec-WebSocket-Protocol")
                                        .and_then(|value| value.to_str().ok());
                                    if !selected_subprotocol_is_exact(
                                        selected,
                                        subprotocol_advertised,
                                    ) {
                                        return Err(Error::WebSocketHandshakeFailed(format!(
                                            "WebSocket peer did not negotiate required subprotocol for {addr}"
                                        )));
                                    }
                                    Ok::<_, Error>((
                                        ws_stream,
                                        subprotocol_advertised.to_string(),
                                    ))
                                })
                                .await
                                .map_err(|_| Error::ConnectionTimeout(addr))??;

                            let (ws_writer, ws_reader) = ws_stream.split();
                            let connection_arc = Arc::new(
                                WebSocketConnection::from_writer_with_runtime(
                                    ws_writer,
                                    addr,
                                    inner.secure,
                                    selected_subprotocol,
                                    None,
                                    inner.lifecycle.writer_queue_capacity,
                                    inner.lifecycle.write_timeout,
                                ),
                            );
                            if inner.closed.load(Ordering::Acquire) {
                                let _ = connection_arc.close().await;
                                return Err(Error::TransportClosed);
                            }

                            let generation = inner
                                .next_connection_generation
                                .fetch_add(1, Ordering::Relaxed);
                            let weak_inner = Arc::downgrade(&inner);
                            let (start_tx, start_rx) = tokio::sync::oneshot::channel();
                            let reader_connection = connection_arc.clone();
                            let reader_key = key.clone();
                            let lifecycle = inner.lifecycle;
                            let reader_task = tokio::time::timeout_at(
                                deadline,
                                inner.tasks.spawn(async move {
                                    if start_rx.await.is_ok() {
                                        Self::run_connection_reader(
                                            weak_inner,
                                            reader_connection,
                                            ws_reader,
                                            generation,
                                            reader_key,
                                            lifecycle,
                                            established_permit,
                                        )
                                        .await;
                                    }
                                }),
                            )
                            .await
                            .map_err(|_| Error::ConnectionTimeout(addr))?
                            .ok_or(Error::TransportClosed)?;

                            let mut connections = match tokio::time::timeout_at(
                                deadline,
                                inner.connections.lock(),
                            )
                            .await
                            {
                                Ok(connections) => connections,
                                Err(_) => {
                                    reader_task.abort();
                                    let _ = connection_arc.close().await;
                                    return Err(Error::ConnectionTimeout(addr));
                                }
                            };
                            connections.insert(
                                key.clone(),
                                WebSocketConnectionRecord {
                                    generation,
                                    connection: connection_arc.clone(),
                                },
                            );
                            drop(connections);
                            if start_tx.send(()).is_err() {
                                reader_task.abort();
                                let mut connections = inner.connections.lock().await;
                                if connections
                                    .get(&key)
                                    .is_some_and(|record| record.generation == generation)
                                {
                                    connections.remove(&key);
                                }
                                drop(connections);
                                let _ = connection_arc.close().await;
                                return Err(Error::TransportClosed);
                            }

                            debug!(
                                "WebSocket client connected to {} (subprotocol={})",
                                addr,
                                connection_arc.subprotocol()
                            );
                            Ok(connection_arc)
                        }
                        .await;
                        coordinator_for_task.complete(
                            &key,
                            &flight,
                            &result,
                            &mut cancellation,
                        );
                        result
                    })
                    .await
            }
        }
    }
}

#[async_trait::async_trait]
impl Transport for WebSocketTransport {
    fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.inner.local_addr)
    }

    async fn send_message(&self, message: Message, destination: SocketAddr) -> Result<()> {
        if self.is_closed() {
            return Err(Error::TransportClosed);
        }
        validate_typed_outbound_message(&message)?;

        debug!(
            "Sending {} message to {}",
            if let Message::Request(ref req) = message {
                safe_method_label(&req.method).to_string()
            } else {
                "response".to_string()
            },
            destination
        );

        #[cfg(feature = "ws")]
        {
            if matches!(message, Message::Response(_)) {
                let existing = {
                    let connections = self.inner.connections.lock().await;
                    select_unambiguous_websocket_flow(&connections, destination)?
                };
                if let Some(connection) = existing {
                    return connection.send_message(&message).await;
                }
            }
            let authority = WebSocketAuthority::for_message(&message, destination);
            let connection = self.connect_to(destination, authority).await?;

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
            let existing = {
                let connections = self.inner.connections.lock().await;
                select_unambiguous_websocket_flow(&connections, destination)?
            };
            if let Some(connection) = existing {
                return connection.send_raw_bytes(bytes).await;
            }
            let connection = self
                .connect_to(destination, WebSocketAuthority::for_address(destination))
                .await?;
            connection.send_raw_bytes(bytes).await
        }

        #[cfg(not(feature = "ws"))]
        Err(Error::NotImplemented(
            "WebSocket transport not implemented".into(),
        ))
    }

    async fn close(&self) -> Result<()> {
        let _close_guard = self.inner.close_gate.lock().await;
        let already_closed = self.inner.closed.swap(true, Ordering::AcqRel);
        self.inner.outbound_dials.close();
        self.inner.inbound_established.close();
        self.inner.outbound_established.close();
        self.inner.tasks.close().await;
        if already_closed {
            return Ok(());
        }

        let connections: Vec<_> = self.inner.connections.lock().await.drain().collect();
        close_websocket_connections(connections).await;

        // Never block shutdown behind a full application event channel.
        let _ = self.inner.events_tx.try_send(TransportEvent::Closed);

        Ok(())
    }

    fn is_closed(&self) -> bool {
        self.inner.closed.load(Ordering::Relaxed)
    }
}

impl fmt::Debug for WebSocketTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "WebSocketTransport({})", self.inner.local_addr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[cfg(feature = "ws")]
    async fn connect_plain(
        transport: &WebSocketTransport,
        destination: SocketAddr,
    ) -> Result<Arc<WebSocketConnection>> {
        transport
            .connect_to(destination, WebSocketAuthority::for_address(destination))
            .await
    }

    #[test]
    fn client_subprotocol_validation_is_fail_closed() {
        assert!(!selected_subprotocol_is_exact(None, "sip"));
        assert!(!selected_subprotocol_is_exact(Some("chat"), "sip"));
        assert!(!selected_subprotocol_is_exact(Some("sips"), "sip"));
        assert!(selected_subprotocol_is_exact(Some("sip"), "sip"));
    }
    use rvoip_sip_core::builder::SimpleRequestBuilder;
    use rvoip_sip_core::types::headers::{HeaderName, HeaderValue, TypedHeader};
    use rvoip_sip_core::{Method, Response, StatusCode};
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

        let invalid_reason =
            Response::new(StatusCode::Ok).with_reason("OK\r\nX-Injected: websocket-reason-secret");

        for message in [Message::Request(request), Message::Response(invalid_reason)] {
            let error = transport
                .send_message(message, destination)
                .await
                .expect_err("typed WS/WSS send must reject unsafe fields");
            assert!(matches!(error, Error::ProtocolError(_)));
            assert!(!error.to_string().contains("X-Injected"));
        }
        transport.close().await.unwrap();
    }

    #[cfg(feature = "ws")]
    #[tokio::test]
    async fn outbound_websocket_handshake_has_end_to_end_deadline() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let destination = listener.local_addr().unwrap();
        let stalled = tokio::spawn(async move {
            let (_stream, _) = listener.accept().await.unwrap();
            std::future::pending::<()>().await;
        });
        let (transport, _events) = WebSocketTransport::bind_with_handshake_config(
            "127.0.0.1:0".parse().unwrap(),
            false,
            None,
            None,
            None,
            HandshakeAdmissionConfig::new(Duration::from_millis(50), 1),
        )
        .await
        .unwrap();

        assert!(matches!(
            connect_plain(&transport, destination).await,
            Err(Error::ConnectionTimeout(address)) if address == destination
        ));
        transport.close().await.unwrap();
        stalled.abort();
    }

    #[cfg(feature = "ws")]
    #[tokio::test]
    async fn close_cancels_and_joins_outbound_websocket_handshake() {
        let listener = tokio::net::TcpListener::bind("127.0.0.1:0").await.unwrap();
        let destination = listener.local_addr().unwrap();
        let (accepted_tx, accepted_rx) = tokio::sync::oneshot::channel();
        let stalled = tokio::spawn(async move {
            let (_stream, _) = listener.accept().await.unwrap();
            let _ = accepted_tx.send(());
            std::future::pending::<()>().await;
        });
        let (transport, _events) = WebSocketTransport::bind_with_handshake_config(
            "127.0.0.1:0".parse().unwrap(),
            false,
            None,
            None,
            None,
            HandshakeAdmissionConfig::new(Duration::from_secs(30), 1),
        )
        .await
        .unwrap();
        let dialing_transport = transport.clone();
        let dialing =
            tokio::spawn(async move { connect_plain(&dialing_transport, destination).await });
        accepted_rx.await.unwrap();
        transport.close().await.unwrap();

        assert!(matches!(
            dialing.await.unwrap(),
            Err(Error::TransportClosed)
        ));
        stalled.abort();
    }

    #[cfg(feature = "ws")]
    #[tokio::test]
    async fn concurrent_websocket_dials_to_one_destination_are_singleflight() {
        let (server, _server_events) =
            WebSocketTransport::bind("127.0.0.1:0".parse().unwrap(), false, None, None, None)
                .await
                .unwrap();
        let destination = server.local_addr().unwrap();
        let (client, _client_events) = WebSocketTransport::bind_with_handshake_config(
            "127.0.0.1:0".parse().unwrap(),
            false,
            None,
            None,
            None,
            HandshakeAdmissionConfig::new(Duration::from_secs(2), 8),
        )
        .await
        .unwrap();

        let (first, second) = tokio::join!(
            connect_plain(&client, destination),
            connect_plain(&client, destination)
        );
        let first = first.unwrap();
        let second = second.unwrap();
        assert!(Arc::ptr_eq(&first, &second));
        tokio::time::sleep(Duration::from_millis(20)).await;
        assert_eq!(client.inner.connections.lock().await.len(), 1);
        assert_eq!(server.inner.connections.lock().await.len(), 1);

        client.close().await.unwrap();
        server.close().await.unwrap();
    }

    #[cfg(feature = "ws")]
    #[tokio::test]
    async fn same_address_different_websocket_authorities_never_share_connection() {
        let (server, _server_events) =
            WebSocketTransport::bind("127.0.0.1:0".parse().unwrap(), false, None, None, None)
                .await
                .unwrap();
        let destination = server.local_addr().unwrap();
        let (client, _client_events) =
            WebSocketTransport::bind("127.0.0.1:0".parse().unwrap(), false, None, None, None)
                .await
                .unwrap();
        let first_authority = WebSocketAuthority {
            host: WebSocketAuthorityHost::Dns("authority-a.example".into()),
            port: destination.port(),
        };
        let second_authority = WebSocketAuthority {
            host: WebSocketAuthorityHost::Dns("authority-b.example".into()),
            port: destination.port(),
        };

        let first = client
            .connect_to(destination, first_authority)
            .await
            .unwrap();
        let second = client
            .connect_to(destination, second_authority)
            .await
            .unwrap();
        assert!(!Arc::ptr_eq(&first, &second));
        assert_eq!(client.inner.connections.lock().await.len(), 2);
        assert!(matches!(
            client
                .send_message(
                    Message::Response(Response::new(StatusCode::Ok)),
                    destination,
                )
                .await,
            Err(Error::InvalidState(_))
        ));

        client.close().await.unwrap();
        server.close().await.unwrap();
    }

    #[cfg(feature = "ws")]
    #[tokio::test]
    async fn response_reuses_sole_authenticated_outbound_websocket_flow() {
        let (server, _server_events) =
            WebSocketTransport::bind("127.0.0.1:0".parse().unwrap(), false, None, None, None)
                .await
                .unwrap();
        let destination = server.local_addr().unwrap();
        let (client, _client_events) =
            WebSocketTransport::bind("127.0.0.1:0".parse().unwrap(), false, None, None, None)
                .await
                .unwrap();
        client
            .connect_to(
                destination,
                WebSocketAuthority {
                    host: WebSocketAuthorityHost::Dns("sip.example.test".into()),
                    port: destination.port(),
                },
            )
            .await
            .unwrap();

        client
            .send_message(
                Message::Response(Response::new(StatusCode::Ok)),
                destination,
            )
            .await
            .unwrap();
        assert_eq!(client.inner.connections.lock().await.len(), 1);

        client.close().await.unwrap();
        server.close().await.unwrap();
    }

    #[cfg(feature = "ws")]
    #[tokio::test]
    async fn websocket_drain_deadline_does_not_multiply_by_connection_count() {
        use tokio_tungstenite::tungstenite::protocol::Role;
        use tokio_tungstenite::WebSocketStream;

        let mut peers = Vec::new();
        let mut sends = Vec::new();
        let mut connections = Vec::new();
        for index in 0..4u16 {
            let (transport_side, blocked_peer) = tokio::io::duplex(1);
            peers.push(blocked_peer);
            let stream = WebSocketStream::from_raw_socket(
                SipWsStream::Test(transport_side),
                Role::Client,
                Some(connection::sip_websocket_config()),
            )
            .await;
            let (writer, _reader) = stream.split();
            let remote_addr: SocketAddr = format!("127.0.0.1:{}", 5100 + index).parse().unwrap();
            let connection = Arc::new(WebSocketConnection::from_writer_with_runtime(
                writer,
                remote_addr,
                false,
                "sip".into(),
                None,
                1,
                Duration::from_millis(40),
            ));
            let sending_connection = connection.clone();
            sends.push(tokio::spawn(async move {
                sending_connection
                    .send_raw_bytes(bytes::Bytes::from(vec![b'x'; 4_096]))
                    .await
            }));
            connections.push((
                WebSocketConnectionKey::outbound(
                    remote_addr,
                    WebSocketAuthority::for_address(remote_addr),
                    1,
                    false,
                ),
                WebSocketConnectionRecord {
                    generation: u64::from(index),
                    connection,
                },
            ));
        }
        tokio::time::sleep(Duration::from_millis(5)).await;

        let started = tokio::time::Instant::now();
        close_websocket_connections(connections).await;
        assert!(
            started.elapsed() < Duration::from_millis(130),
            "connection drain was serialized: {:?}",
            started.elapsed()
        );
        for send in sends {
            assert!(send.await.unwrap().is_err());
        }
        drop(peers);
    }

    #[cfg(feature = "wss")]
    #[test]
    fn wss_http_host_and_sni_share_one_normalized_authority() {
        let authority = WebSocketAuthority {
            host: WebSocketAuthorityHost::Dns("sip.example.test".into()),
            port: 7443,
        };
        assert_eq!(authority.http_authority(), "sip.example.test:7443");
        let server_name = authority.server_name().unwrap();
        assert!(matches!(
            server_name,
            tokio_rustls::rustls::pki_types::ServerName::DnsName(name)
                if name.as_ref() == "sip.example.test"
        ));
    }

    #[cfg(feature = "ws")]
    #[tokio::test]
    async fn websocket_connection_registry_lock_is_inside_dial_deadline() {
        let (client, _events) = WebSocketTransport::bind_with_handshake_config(
            "127.0.0.1:0".parse().unwrap(),
            false,
            None,
            None,
            None,
            HandshakeAdmissionConfig::new(Duration::from_millis(40), 1),
        )
        .await
        .unwrap();
        let destination = "127.0.0.1:9".parse().unwrap();
        let _registry_guard = client.inner.connections.lock().await;
        let started = tokio::time::Instant::now();
        assert!(matches!(
            connect_plain(&client, destination).await,
            Err(Error::ConnectionTimeout(address)) if address == destination
        ));
        assert!(started.elapsed() < Duration::from_millis(250));
    }

    #[cfg(feature = "ws")]
    #[tokio::test]
    async fn websocket_established_caps_hold_for_both_directions() {
        let config = HandshakeAdmissionConfig::new(Duration::from_secs(1), 1);
        let (server, _server_events) = WebSocketTransport::bind_with_handshake_config(
            "127.0.0.1:0".parse().unwrap(),
            false,
            None,
            None,
            None,
            config,
        )
        .await
        .unwrap();
        let destination = server.local_addr().unwrap();
        let (first_client, _events) = WebSocketTransport::bind_with_handshake_config(
            "127.0.0.1:0".parse().unwrap(),
            false,
            None,
            None,
            None,
            config,
        )
        .await
        .unwrap();

        connect_plain(&first_client, destination).await.unwrap();
        let second_authority = WebSocketAuthority {
            host: WebSocketAuthorityHost::Dns("second.example".into()),
            port: destination.port(),
        };
        assert!(matches!(
            first_client.connect_to(destination, second_authority).await,
            Err(Error::ConnectionPoolExhausted)
        ));

        let (second_client, _events) = WebSocketTransport::bind_with_handshake_config(
            "127.0.0.1:0".parse().unwrap(),
            false,
            None,
            None,
            None,
            config,
        )
        .await
        .unwrap();
        let _second_flow = connect_plain(&second_client, destination).await;
        tokio::time::sleep(Duration::from_millis(30)).await;
        assert_eq!(server.inner.connections.lock().await.len(), 1);

        second_client.close().await.unwrap();
        first_client.close().await.unwrap();
        server.close().await.unwrap();
    }

    #[cfg(feature = "ws")]
    #[tokio::test]
    async fn idle_websocket_connections_are_evicted_and_release_permits() {
        let config = HandshakeAdmissionConfig::new(Duration::from_millis(50), 2);
        let (server, _server_events) = WebSocketTransport::bind_with_handshake_config(
            "127.0.0.1:0".parse().unwrap(),
            false,
            None,
            None,
            None,
            config,
        )
        .await
        .unwrap();
        let destination = server.local_addr().unwrap();
        let (client, _events) = WebSocketTransport::bind_with_handshake_config(
            "127.0.0.1:0".parse().unwrap(),
            false,
            None,
            None,
            None,
            config,
        )
        .await
        .unwrap();
        connect_plain(&client, destination).await.unwrap();
        assert_eq!(client.inner.connections.lock().await.len(), 1);

        tokio::time::sleep(Duration::from_millis(1_650)).await;
        assert!(client.inner.connections.lock().await.is_empty());
        assert!(server.inner.connections.lock().await.is_empty());

        client.close().await.unwrap();
        server.close().await.unwrap();
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
