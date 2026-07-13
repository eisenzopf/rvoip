mod connection;
mod listener;
pub mod pool;

pub use connection::{ReceivedFrame, TcpConnection};
pub use listener::TcpListener;
pub use pool::{ConnectionPool, PoolConfig};

use crate::error::{Error, Result};
use crate::transport::{
    runtime::{DialAdmission, OutboundDialCoordinator, TransportTaskSet},
    safe_method_label, validate_typed_outbound_message, Transport, TransportEvent, TransportFlowId,
    TransportRoute, TransportType,
};
use bytes::Bytes;
use rvoip_sip_core::Message;
use std::fmt;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::mpsc;
use tracing::{debug, error, info, trace, warn};

// Default channel capacity
const DEFAULT_CHANNEL_CAPACITY: usize = 1000;

/// TCP transport for SIP messages with connection pooling
#[derive(Clone)]
pub struct TcpTransport {
    inner: Arc<TcpTransportInner>,
}

struct TcpTransportInner {
    listener: tokio::sync::Mutex<Option<Arc<TcpListener>>>,
    local_addr: SocketAddr,
    connection_pool: ConnectionPool,
    closed: AtomicBool,
    events_tx: mpsc::Sender<TransportEvent>,
    outbound_dials: Arc<OutboundDialCoordinator<SocketAddr>>,
    tasks: Arc<TransportTaskSet>,
}

impl TcpTransport {
    /// Creates a new TCP transport bound to the specified address
    pub async fn bind(
        addr: SocketAddr,
        channel_capacity: Option<usize>,
        pool_config: Option<PoolConfig>,
    ) -> Result<(Self, mpsc::Receiver<TransportEvent>)> {
        // Create the event channel
        let capacity = channel_capacity.unwrap_or(DEFAULT_CHANNEL_CAPACITY);
        let (events_tx, events_rx) = mpsc::channel(capacity);

        // Create the TCP listener
        let listener = TcpListener::bind(addr).await?;
        let local_addr = listener.local_addr()?;
        info!("SIP TCP transport bound to {}", local_addr);

        // Create the connection pool with the specified configuration or defaults
        let config = pool_config.unwrap_or_default();
        let outbound_dial_limit = config.max_connections.max(1);
        let connection_pool = ConnectionPool::new(config);
        let tasks = TransportTaskSet::new();

        // Create the transport
        let transport = TcpTransport {
            inner: Arc::new(TcpTransportInner {
                listener: tokio::sync::Mutex::new(Some(Arc::new(listener))),
                local_addr,
                connection_pool,
                closed: AtomicBool::new(false),
                events_tx: events_tx.clone(),
                outbound_dials: OutboundDialCoordinator::new(
                    outbound_dial_limit,
                    outbound_dial_limit.saturating_mul(2),
                    std::time::Duration::from_millis(100),
                ),
                tasks: tasks.clone(),
            }),
        };

        // Start the accept loop to accept incoming connections
        transport.spawn_accept_loop().await;
        let _ = tasks
            .spawn(transport.inner.connection_pool.clone().run_cleanup())
            .await;

        Ok((transport, events_rx))
    }

    /// Spawns a task to accept incoming connections
    async fn spawn_accept_loop(&self) {
        let weak_inner = Arc::downgrade(&self.inner);
        let listener_clone = self
            .inner
            .listener
            .lock()
            .await
            .clone()
            .expect("listener is present before accept supervision starts");

        let _ = self
            .inner
            .tasks
            .spawn(async move {
                loop {
                    let Some(inner) = weak_inner.upgrade() else {
                        break;
                    };
                    if inner.closed.load(Ordering::Relaxed) {
                        break;
                    }
                    drop(inner);

                    // Accept a new connection
                    match listener_clone.accept().await {
                        Ok((stream, peer_addr)) => {
                            debug!("Accepted TCP connection from {}", peer_addr);

                            // Create a connection object
                            let connection = match TcpConnection::from_stream(stream, peer_addr) {
                                Ok(conn) => conn,
                                Err(e) => {
                                    error!("Failed to create connection from stream: {}", e);
                                    if let Some(inner) = weak_inner.upgrade() {
                                        let events_tx = inner.events_tx.clone();
                                        drop(inner);
                                        let _ = events_tx
                                            .send(TransportEvent::Error {
                                                error: format!("Connection setup error: {}", e),
                                            })
                                            .await;
                                    }
                                    continue;
                                }
                            };

                            // Publish the connection into the pool so that outbound
                            // writes (responses, and RFC 5626 CRLFCRLF keep-alives
                            // via `send_raw`) can find it, then spawn the unified
                            // reader.
                            let arc = Arc::new(connection);
                            let Some(inner) = weak_inner.upgrade() else {
                                let _ = arc.close().await;
                                break;
                            };
                            let pool = inner.connection_pool.clone();
                            let events_tx = inner.events_tx.clone();
                            let tasks = inner.tasks.clone();
                            drop(inner);
                            if let Err(error) = pool.add_connection(peer_addr, arc.clone()).await {
                                warn!(
                                    source = %peer_addr,
                                    error_class = "connection-limit",
                                    "Rejecting inbound TCP connection at pool capacity"
                                );
                                let _ = events_tx
                                    .send(TransportEvent::Error {
                                        error: error.to_string(),
                                    })
                                    .await;
                                continue;
                            }
                            Self::spawn_connection_handler_task(tasks, weak_inner.clone(), arc)
                                .await;
                        }
                        Err(e) => {
                            let Some(inner) = weak_inner.upgrade() else {
                                break;
                            };
                            if inner.closed.load(Ordering::Relaxed) {
                                break;
                            }
                            let events_tx = inner.events_tx.clone();
                            drop(inner);

                            error!("Error accepting TCP connection: {}", e);
                            let _ = events_tx
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
                info!("TCP accept loop terminated");
                if let Some(inner) = weak_inner.upgrade() {
                    let events_tx = inner.events_tx.clone();
                    drop(inner);
                    let _ = events_tx.send(TransportEvent::Closed).await;
                }
            })
            .await;
    }

    /// Spawns a handler that drains frames (SIP messages and RFC 5626
    /// keep-alive frames) off a pooled connection until EOF / error.
    /// Used by both inbound-accepted and outbound-dialled connections
    /// so that both directions see keep-alive pongs and emit
    /// `ConnectionClosed` before pool eviction.
    async fn spawn_connection_handler_task(
        tasks: Arc<TransportTaskSet>,
        weak_inner: std::sync::Weak<TcpTransportInner>,
        connection: Arc<TcpConnection>,
    ) {
        let peer_addr = connection.peer_addr();
        let flow_id = connection.flow_id();

        let _ = tasks
            .spawn(async move {
                loop {
                    let Some(inner) = weak_inner.upgrade() else {
                        break;
                    };
                    if inner.closed.load(Ordering::Relaxed) {
                        break;
                    }
                    drop(inner);

                    match connection.receive_frame().await {
                        Ok(Some(ReceivedFrame::Message(message, raw_bytes))) => {
                            debug!("Received SIP message from {}", peer_addr);

                            let local_addr = match connection.local_addr() {
                                Ok(addr) => addr,
                                Err(e) => {
                                    error!("Failed to get local address: {}", e);
                                    break;
                                }
                            };

                            let event = TransportEvent::MessageReceived {
                                message,
                                source: peer_addr,
                                destination: local_addr,
                                transport_type: TransportType::Tcp,
                                flow_id: Some(flow_id),
                                raw_bytes: Some(raw_bytes),
                                timing: None,
                                connection_metadata: None,
                            };

                            let Some(inner) = weak_inner.upgrade() else {
                                break;
                            };
                            let events_tx = inner.events_tx.clone();
                            drop(inner);
                            if let Err(e) = events_tx.send(event).await {
                                error!("Error sending event: {}", e);
                                break;
                            }
                        }
                        Ok(Some(ReceivedFrame::KeepAlivePong)) => {
                            let local_addr = connection.local_addr().unwrap_or(peer_addr);
                            let Some(inner) = weak_inner.upgrade() else {
                                break;
                            };
                            let events_tx = inner.events_tx.clone();
                            drop(inner);
                            let _ = events_tx
                                .send(TransportEvent::KeepAlivePongReceived {
                                    source: peer_addr,
                                    destination: local_addr,
                                    flow_id: Some(flow_id),
                                })
                                .await;
                        }
                        Ok(Some(ReceivedFrame::KeepAlivePing)) => {
                            // Peer-initiated ping. RFC 5626 §3.5.1 says reply
                            // with a single CRLF pong. Keep-alive frames are
                            // best-effort; log-and-drop on send failure.
                            if let Err(e) = connection.send_raw_bytes(b"\r\n").await {
                                debug!("Failed to send CRLF pong to {}: {}", peer_addr, e);
                            }
                        }
                        Ok(None) => {
                            info!("Connection from {} closed gracefully", peer_addr);
                            break;
                        }
                        Err(e) => {
                            let Some(inner) = weak_inner.upgrade() else {
                                break;
                            };
                            if inner.closed.load(Ordering::Relaxed) || connection.is_closed() {
                                break;
                            }
                            let events_tx = inner.events_tx.clone();
                            drop(inner);

                            error!("Error reading from connection {}: {}", peer_addr, e);
                            let _ = events_tx
                                .send(TransportEvent::Error {
                                    error: format!("Connection error from {}: {}", peer_addr, e),
                                })
                                .await;
                            break;
                        }
                    }
                }

                // Emit ConnectionClosed *before* the pool eviction so any
                // downstream observer (e.g. RFC 5626 OutboundFlow) can see
                // the lifecycle event before a subsequent `has_connection_to`
                // query returns false.
                let Some(inner) = weak_inner.upgrade() else {
                    return;
                };
                let events_tx = inner.events_tx.clone();
                let pool = inner.connection_pool.clone();
                drop(inner);
                let _ = events_tx
                    .send(TransportEvent::ConnectionClosed {
                        remote_addr: peer_addr,
                        transport_type: TransportType::Tcp,
                        flow_id: Some(flow_id),
                    })
                    .await;

                pool.remove_connection_for_flow(&peer_addr, flow_id).await;
            })
            .await;
    }

    /// Connects to a remote address and returns a connection
    async fn connect_to(&self, addr: SocketAddr) -> Result<Arc<TcpConnection>> {
        // Check if there's already a connection in the pool
        if let Some(conn) = self.inner.connection_pool.get_connection(&addr).await {
            trace!("Reusing existing connection to {}", addr);
            return Ok(conn);
        }

        let deadline = tokio::time::Instant::now() + std::time::Duration::from_secs(10);
        let coordinator = self.inner.outbound_dials.clone();
        match coordinator.begin(addr)? {
            DialAdmission::Follower { outcome, .. } => {
                OutboundDialCoordinator::<SocketAddr>::wait(outcome, deadline, addr).await?;
                self.inner
                    .connection_pool
                    .get_connection(&addr)
                    .await
                    .ok_or(Error::TransportClosed)
            }
            DialAdmission::Leader {
                key,
                flight,
                _pending,
                cancellation,
            } => {
                let weak_inner = Arc::downgrade(&self.inner);
                let coordinator_for_task = coordinator.clone();
                self.inner
                    .tasks
                    .run(async move {
                        let mut cancellation = cancellation;
                        let _pending = _pending;
                        let result = async {
                            let _handshake = coordinator_for_task
                                .acquire_handshake(deadline, addr)
                                .await?;
                            let Some(inner) = weak_inner.upgrade() else {
                                return Err(Error::TransportClosed);
                            };
                            if inner.closed.load(Ordering::Acquire) || inner.tasks.is_closing() {
                                return Err(Error::TransportClosed);
                            }
                            let pool = inner.connection_pool.clone();
                            drop(inner);
                            if let Some(connection) = pool.get_connection(&addr).await {
                                return Ok(connection);
                            }

                            debug!("Creating new connection to {}", addr);
                            let connection =
                                tokio::time::timeout_at(deadline, TcpConnection::connect(addr))
                                    .await
                                    .map_err(|_| Error::ConnectionTimeout(addr))??;
                            let Some(inner) = weak_inner.upgrade() else {
                                let _ = connection.close().await;
                                return Err(Error::TransportClosed);
                            };
                            if inner.closed.load(Ordering::Acquire) || inner.tasks.is_closing() {
                                drop(inner);
                                let _ = connection.close().await;
                                return Err(Error::TransportClosed);
                            }
                            let pool = inner.connection_pool.clone();
                            let tasks = inner.tasks.clone();
                            drop(inner);

                            let connection = Arc::new(connection);
                            pool.add_connection(addr, connection.clone()).await?;
                            Self::spawn_connection_handler_task(
                                tasks,
                                weak_inner.clone(),
                                connection.clone(),
                            )
                            .await;
                            Ok(connection)
                        }
                        .await;
                        coordinator_for_task.complete(&key, &flight, &result, &mut cancellation);
                        result
                    })
                    .await
            }
        }
    }
}

#[async_trait::async_trait]
impl Transport for TcpTransport {
    fn local_addr(&self) -> Result<SocketAddr> {
        Ok(self.inner.local_addr)
    }

    async fn send_message(&self, message: Message, destination: SocketAddr) -> Result<()> {
        self.send_message_via(message, TransportRoute::new(destination))
            .await
    }

    async fn send_message_via(&self, message: Message, route: TransportRoute) -> Result<()> {
        self.send_message_on_route(message, route).await.map(|_| ())
    }

    async fn prepare_message_route(
        &self,
        message: &Message,
        mut route: TransportRoute,
    ) -> Result<TransportRoute> {
        if self.is_closed() {
            return Err(Error::TransportClosed);
        }
        validate_typed_outbound_message(message)?;
        let connection = if let Some(flow_id) = route.flow_id {
            self.inner
                .connection_pool
                .get_connection_for_flow(&route.destination, flow_id)
                .await
                .ok_or_else(|| {
                    Error::InvalidState(format!(
                        "TCP flow is no longer active for {}",
                        route.destination
                    ))
                })?
        } else {
            if matches!(message, Message::Response(_)) {
                return Err(Error::InvalidState(
                    "TCP responses require the exact ingress flow".into(),
                ));
            }
            self.connect_to(route.destination).await?
        };
        route.transport_type = Some(TransportType::Tcp);
        route.flow_id = Some(connection.flow_id());
        Ok(route)
    }

    async fn send_message_on_route(
        &self,
        message: Message,
        mut route: TransportRoute,
    ) -> Result<TransportRoute> {
        if self.is_closed() {
            return Err(Error::TransportClosed);
        }
        validate_typed_outbound_message(&message)?;
        let destination = route.destination;

        debug!(
            "Sending {} message to {}",
            if let Message::Request(ref req) = message {
                safe_method_label(&req.method).to_string()
            } else {
                "response".to_string()
            },
            destination
        );

        let connection = if let Some(flow_id) = route.flow_id {
            self.inner
                .connection_pool
                .get_connection_for_flow(&destination, flow_id)
                .await
                .ok_or_else(|| {
                    Error::InvalidState(format!("TCP flow is no longer active for {destination}"))
                })?
        } else {
            if matches!(message, Message::Response(_)) {
                return Err(Error::InvalidState(
                    "TCP responses require the exact ingress flow".into(),
                ));
            }
            self.connect_to(destination).await?
        };

        // Send the message
        connection.send_message(&message).await?;
        route.transport_type = Some(TransportType::Tcp);
        route.flow_id = Some(connection.flow_id());
        Ok(route)
    }

    async fn send_message_raw(&self, bytes: Bytes, destination: SocketAddr) -> Result<()> {
        self.send_message_raw_via(bytes, TransportRoute::new(destination))
            .await
    }

    async fn send_message_raw_via(&self, bytes: Bytes, route: TransportRoute) -> Result<()> {
        if self.is_closed() {
            return Err(Error::TransportClosed);
        }
        let destination = route.destination;
        debug!(
            "TCP: sending {} pre-built bytes to {}",
            bytes.len(),
            destination
        );
        // Resolve or open a connection — `send_message_raw` is the
        // general-purpose verbatim-bytes path (unlike `send_raw`,
        // which is RFC 5626 keep-alive on already-open flows only).
        let connection = if let Some(flow_id) = route.flow_id {
            self.inner
                .connection_pool
                .get_connection_for_flow(&destination, flow_id)
                .await
                .ok_or_else(|| {
                    Error::InvalidState(format!("TCP flow is no longer active for {destination}"))
                })?
        } else {
            self.connect_to(destination).await?
        };
        connection.send_raw_bytes(&bytes).await
    }

    async fn close(&self) -> Result<()> {
        // Set the closed flag to prevent new operations
        self.inner.closed.store(true, Ordering::Relaxed);
        self.inner.outbound_dials.close();
        self.inner.tasks.close().await;
        self.inner.listener.lock().await.take();

        // Close all connections in the pool
        self.inner.connection_pool.close_all().await;

        Ok(())
    }

    fn is_closed(&self) -> bool {
        self.inner.closed.load(Ordering::Relaxed)
    }

    fn supports_tcp(&self) -> bool {
        true
    }

    fn has_connection_to(&self, remote_addr: SocketAddr) -> bool {
        self.inner.connection_pool.has_connection(&remote_addr)
    }

    fn flow_id_for_route(&self, route: &TransportRoute) -> Option<TransportFlowId> {
        let flow_id = self.inner.connection_pool.flow_id_for(&route.destination)?;
        route.flow_id.map_or(Some(flow_id), |expected| {
            (expected == flow_id).then_some(flow_id)
        })
    }

    async fn resolve_flow_id_for_route(&self, route: &TransportRoute) -> Option<TransportFlowId> {
        let flow_id = self
            .inner
            .connection_pool
            .resolve_flow_id_for(&route.destination)
            .await?;
        route.flow_id.map_or(Some(flow_id), |expected| {
            (expected == flow_id).then_some(flow_id)
        })
    }

    async fn send_raw(&self, destination: SocketAddr, data: Bytes) -> Result<()> {
        self.send_raw_via(TransportRoute::new(destination), data)
            .await
    }

    async fn send_raw_via(&self, route: TransportRoute, data: Bytes) -> Result<()> {
        if self.is_closed() {
            return Err(Error::TransportClosed);
        }

        // RFC 5626 keep-alive: only reuse an existing pooled connection
        // — never open a fresh TCP dial for a bare-bytes write. If the
        // flow is gone, the caller (typically a ping task in dialog-
        // core) terminates; a fresh flow is the upper layer's job.
        let destination = route.destination;
        let connection = match route.flow_id {
            Some(flow_id) => {
                self.inner
                    .connection_pool
                    .get_connection_for_flow(&destination, flow_id)
                    .await
            }
            None => {
                self.inner
                    .connection_pool
                    .get_connection(&destination)
                    .await
            }
        };
        let Some(connection) = connection else {
            return Err(Error::InvalidState(format!(
                "No active TCP connection to {} for send_raw",
                destination
            )));
        };

        connection.send_raw_bytes(&data).await
    }
}

impl fmt::Debug for TcpTransport {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "TcpTransport({})", self.inner.local_addr)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rvoip_sip_core::builder::{SimpleRequestBuilder, SimpleResponseBuilder};
    use rvoip_sip_core::types::headers::{HeaderName, HeaderValue, TypedHeader};
    use rvoip_sip_core::{Method, Response, StatusCode};
    use std::collections::HashSet;
    use tokio::time::Duration;

    #[tokio::test]
    async fn test_tcp_transport_bind() {
        let config = PoolConfig {
            max_connections: 10,
            idle_timeout: Duration::from_secs(10),
        };

        let (transport, _rx) =
            TcpTransport::bind("127.0.0.1:0".parse().unwrap(), Some(10), Some(config))
                .await
                .unwrap();

        let addr = transport.local_addr().unwrap();
        assert!(addr.port() > 0);

        transport.close().await.unwrap();
        assert!(transport.is_closed());
    }

    #[tokio::test]
    async fn typed_tcp_send_rejects_auth_before_opening_a_connection() {
        let (transport, _rx) = TcpTransport::bind("127.0.0.1:0".parse().unwrap(), Some(10), None)
            .await
            .unwrap();
        let destination = "127.0.0.1:9".parse().unwrap();
        let mut request = SimpleRequestBuilder::new(Method::Options, "sip:example.com")
            .unwrap()
            .build();
        request.headers.push(TypedHeader::Other(
            HeaderName::Other("AUTHORIZATION".into()),
            HeaderValue::Raw(b"Bearer safe\r\nX-Injected: tcp".to_vec()),
        ));

        let invalid_reason =
            Response::new(StatusCode::Ok).with_reason("OK\r\nX-Injected: tcp-reason-secret");

        for message in [Message::Request(request), Message::Response(invalid_reason)] {
            let error = transport
                .send_message(message, destination)
                .await
                .expect_err("typed TCP send must reject unsafe fields");
            assert!(matches!(error, Error::ProtocolError(_)));
            assert!(!transport.has_connection_to(destination));
            assert!(!error.to_string().contains("X-Injected"));
        }
        transport.close().await.unwrap();
    }

    #[tokio::test]
    async fn test_tcp_transport_send_receive() {
        // Start a server transport
        let (server_transport, mut server_rx) =
            TcpTransport::bind("127.0.0.1:0".parse().unwrap(), Some(10), None)
                .await
                .unwrap();

        let server_addr = server_transport.local_addr().unwrap();

        // Start a client transport
        let (client_transport, _client_rx) =
            TcpTransport::bind("127.0.0.1:0".parse().unwrap(), Some(10), None)
                .await
                .unwrap();

        // Create a test SIP message
        let request = SimpleRequestBuilder::new(Method::Register, "sip:example.com")
            .unwrap()
            .from("alice", "sip:alice@example.com", Some("tag1"))
            .to("bob", "sip:bob@example.com", None)
            .call_id("call1@example.com")
            .cseq(1)
            .build();

        // Send the message
        client_transport
            .send_message(request.into(), server_addr)
            .await
            .unwrap();

        // Receive the message
        let event = tokio::time::timeout(Duration::from_secs(5), server_rx.recv())
            .await
            .unwrap()
            .unwrap();

        match event {
            TransportEvent::MessageReceived {
                message,
                source: _,
                destination,
                ..
            } => {
                assert_eq!(destination, server_addr);
                assert!(message.is_request());
                if let Message::Request(req) = message {
                    assert_eq!(req.method(), Method::Register);
                } else {
                    panic!("Expected a request");
                }
            }
            _ => panic!("Expected MessageReceived event"),
        }

        // Clean up
        client_transport.close().await.unwrap();
        server_transport.close().await.unwrap();
    }

    /// `has_connection_to` reports true for an address in the pool and
    /// false after the connection is dropped.
    #[tokio::test]
    async fn has_connection_to_reflects_pool() {
        let (server, _server_rx) =
            TcpTransport::bind("127.0.0.1:0".parse().unwrap(), Some(10), None)
                .await
                .unwrap();
        let server_addr = server.local_addr().unwrap();

        let (client, _client_rx) =
            TcpTransport::bind("127.0.0.1:0".parse().unwrap(), Some(10), None)
                .await
                .unwrap();

        // No connection yet.
        assert!(!client.has_connection_to(server_addr));

        // Dial by sending a SIP message — TCP auto-connects through the pool.
        let req = SimpleRequestBuilder::new(Method::Register, "sip:example.com")
            .unwrap()
            .from("alice", "sip:alice@example.com", Some("tag1"))
            .to("bob", "sip:bob@example.com", None)
            .call_id("has-conn@example.com")
            .cseq(1)
            .build();
        client.send_message(req.into(), server_addr).await.unwrap();

        assert!(client.has_connection_to(server_addr));

        client.close().await.unwrap();
        server.close().await.unwrap();
    }

    #[tokio::test]
    async fn pool_limit_rejects_new_peer_without_orphaning_old_exact_flow() {
        let config = PoolConfig {
            max_connections: 1,
            idle_timeout: Duration::from_secs(10),
        };
        let (server, mut server_events) =
            TcpTransport::bind("127.0.0.1:0".parse().unwrap(), Some(16), Some(config))
                .await
                .unwrap();
        let server_addr = server.local_addr().unwrap();
        let (first_client, mut first_events) =
            TcpTransport::bind("127.0.0.1:0".parse().unwrap(), Some(16), None)
                .await
                .unwrap();
        let (second_client, _second_events) =
            TcpTransport::bind("127.0.0.1:0".parse().unwrap(), Some(16), None)
                .await
                .unwrap();

        let first_request = SimpleRequestBuilder::new(Method::Options, "sip:first.example")
            .unwrap()
            .from("alice", "sip:alice@example.com", Some("first"))
            .to("service", "sip:first.example", None)
            .call_id("tcp-cap-first")
            .cseq(1)
            .build();
        let first_response =
            SimpleResponseBuilder::response_from_request(&first_request, StatusCode::Ok, None)
                .build();
        first_client
            .send_message(Message::Request(first_request), server_addr)
            .await
            .unwrap();
        let first_route = match tokio::time::timeout(Duration::from_secs(2), server_events.recv())
            .await
            .unwrap()
            .unwrap()
        {
            TransportEvent::MessageReceived {
                source,
                flow_id: Some(flow_id),
                ..
            } => TransportRoute::new(source)
                .with_transport_type(TransportType::Tcp)
                .with_flow_id(flow_id),
            event => panic!("expected first flow-bearing TCP request, got {event:?}"),
        };

        let second_request = SimpleRequestBuilder::new(Method::Options, "sip:second.example")
            .unwrap()
            .from("alice", "sip:alice@example.com", Some("second"))
            .to("service", "sip:second.example", None)
            .call_id("tcp-cap-second")
            .cseq(1)
            .build();
        second_client
            .send_message(Message::Request(second_request), server_addr)
            .await
            .unwrap();
        let rejected = tokio::time::timeout(Duration::from_secs(2), async {
            loop {
                if matches!(
                    server_events.recv().await,
                    Some(TransportEvent::Error { .. })
                ) {
                    break;
                }
            }
        })
        .await;
        assert!(rejected.is_ok(), "second peer was not rejected at the cap");

        assert_eq!(
            server.flow_id_for_route(&first_route),
            first_route.flow_id,
            "rejecting a new peer must retain the original exact route"
        );
        server
            .send_message_via(Message::Response(first_response), first_route)
            .await
            .unwrap();
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(2), first_events.recv())
                .await
                .unwrap()
                .unwrap(),
            TransportEvent::MessageReceived {
                message: Message::Response(_),
                ..
            }
        ));

        first_client.close().await.unwrap();
        second_client.close().await.unwrap();
        server.close().await.unwrap();
    }

    #[tokio::test]
    async fn concurrent_sends_singleflight_one_tcp_connection() {
        const SENDERS: usize = 16;
        let (server, mut server_events) =
            TcpTransport::bind("127.0.0.1:0".parse().unwrap(), Some(64), None)
                .await
                .unwrap();
        let destination = server.local_addr().unwrap();
        let (client, _client_events) =
            TcpTransport::bind("127.0.0.1:0".parse().unwrap(), Some(64), None)
                .await
                .unwrap();
        let barrier = Arc::new(tokio::sync::Barrier::new(SENDERS));
        let mut sends = tokio::task::JoinSet::new();
        for index in 0..SENDERS {
            let client = client.clone();
            let barrier = barrier.clone();
            sends.spawn(async move {
                let call_id = format!("tcp-singleflight-{index}");
                let request = SimpleRequestBuilder::new(Method::Options, "sip:singleflight.test")
                    .unwrap()
                    .from("alice", "sip:alice@example.test", Some("tag"))
                    .to("service", "sip:singleflight.test", None)
                    .call_id(&call_id)
                    .cseq(1)
                    .build();
                barrier.wait().await;
                client
                    .send_message(Message::Request(request), destination)
                    .await
            });
        }
        while let Some(result) = sends.join_next().await {
            result.unwrap().unwrap();
        }

        let mut flows = HashSet::new();
        for _ in 0..SENDERS {
            match tokio::time::timeout(Duration::from_secs(2), server_events.recv())
                .await
                .unwrap()
                .unwrap()
            {
                TransportEvent::MessageReceived {
                    flow_id: Some(flow_id),
                    ..
                } => {
                    flows.insert(flow_id);
                }
                event => panic!("expected flow-bearing TCP request, got {event:?}"),
            }
        }
        assert_eq!(flows.len(), 1, "singleflight opened more than one socket");

        client.close().await.unwrap();
        server.close().await.unwrap();
    }

    #[tokio::test]
    async fn close_releases_listener_and_prevents_post_close_events() {
        let (server, mut server_events) =
            TcpTransport::bind("127.0.0.1:0".parse().unwrap(), Some(16), None)
                .await
                .unwrap();
        let address = server.local_addr().unwrap();
        let (client, _client_events) =
            TcpTransport::bind("127.0.0.1:0".parse().unwrap(), Some(16), None)
                .await
                .unwrap();
        let request = SimpleRequestBuilder::new(Method::Options, "sip:close-boundary.test")
            .unwrap()
            .from("alice", "sip:alice@example.test", Some("tag"))
            .to("service", "sip:close-boundary.test", None)
            .call_id("tcp-close-boundary")
            .cseq(1)
            .build();
        client
            .send_message(Message::Request(request), address)
            .await
            .unwrap();
        assert!(matches!(
            tokio::time::timeout(Duration::from_secs(1), server_events.recv())
                .await
                .unwrap()
                .unwrap(),
            TransportEvent::MessageReceived { .. }
        ));

        server.close().await.unwrap();
        client.close().await.unwrap();
        tokio::task::yield_now().await;
        assert!(matches!(
            server_events.try_recv(),
            Err(tokio::sync::mpsc::error::TryRecvError::Empty)
        ));

        let (replacement, _replacement_events) =
            TcpTransport::bind(address, Some(4), None).await.unwrap();
        replacement.close().await.unwrap();
    }

    #[tokio::test]
    async fn dropping_transport_without_close_releases_listener() {
        let (transport, events) = TcpTransport::bind("127.0.0.1:0".parse().unwrap(), Some(4), None)
            .await
            .unwrap();
        let address = transport.local_addr().unwrap();
        drop(events);
        drop(transport);
        tokio::task::yield_now().await;

        let (replacement, _replacement_events) =
            TcpTransport::bind(address, Some(4), None).await.unwrap();
        replacement.close().await.unwrap();
    }

    /// RFC 5626: `send_raw` writes bare bytes over the pooled TCP
    /// connection. A `\r\n\r\n` ping from the client triggers the
    /// server's read-loop auto-pong (CRLF), which the client observes
    /// as `KeepAlivePongReceived`.
    #[tokio::test]
    async fn send_raw_triggers_server_pong() {
        let (server, _server_rx) =
            TcpTransport::bind("127.0.0.1:0".parse().unwrap(), Some(10), None)
                .await
                .unwrap();
        let server_addr = server.local_addr().unwrap();

        let (client, mut client_rx) =
            TcpTransport::bind("127.0.0.1:0".parse().unwrap(), Some(10), None)
                .await
                .unwrap();

        // Warm-up REGISTER so there's a pooled TCP connection.
        let req = SimpleRequestBuilder::new(Method::Register, "sip:example.com")
            .unwrap()
            .from("alice", "sip:alice@example.com", Some("tag1"))
            .to("bob", "sip:bob@example.com", None)
            .call_id("warmup@example.com")
            .cseq(1)
            .build();
        client.send_message(req.into(), server_addr).await.unwrap();

        // Ping → pong.
        client
            .send_raw(server_addr, bytes::Bytes::from_static(b"\r\n\r\n"))
            .await
            .unwrap();

        // Client should see the server's CRLF pong as a
        // KeepAlivePongReceived event.
        let mut saw_pong = false;
        for _ in 0..20 {
            if let Ok(Some(event)) =
                tokio::time::timeout(Duration::from_millis(500), client_rx.recv()).await
            {
                if matches!(event, TransportEvent::KeepAlivePongReceived { .. }) {
                    saw_pong = true;
                    break;
                }
            }
        }
        assert!(saw_pong, "client never observed KeepAlivePongReceived");

        client.close().await.unwrap();
        server.close().await.unwrap();
    }

    /// `ConnectionClosed` is emitted when the peer drops the TCP
    /// connection, *before* the per-address entry is evicted from the
    /// pool (so observers can correlate the drop with flow state).
    #[tokio::test]
    async fn connection_closed_emits_before_pool_eviction() {
        let (server, _server_rx) =
            TcpTransport::bind("127.0.0.1:0".parse().unwrap(), Some(10), None)
                .await
                .unwrap();
        let server_addr = server.local_addr().unwrap();

        let (client, mut client_rx) =
            TcpTransport::bind("127.0.0.1:0".parse().unwrap(), Some(10), None)
                .await
                .unwrap();

        // Establish a connection.
        let req = SimpleRequestBuilder::new(Method::Register, "sip:example.com")
            .unwrap()
            .from("alice", "sip:alice@example.com", Some("tag1"))
            .to("bob", "sip:bob@example.com", None)
            .call_id("closed@example.com")
            .cseq(1)
            .build();
        client.send_message(req.into(), server_addr).await.unwrap();
        assert!(client.has_connection_to(server_addr));

        // Force-close the server side. The client's spawned reader
        // should see EOF and emit ConnectionClosed.
        server.close().await.unwrap();

        let mut saw_closed = false;
        for _ in 0..40 {
            if let Ok(Some(event)) =
                tokio::time::timeout(Duration::from_millis(200), client_rx.recv()).await
            {
                if matches!(
                    event,
                    TransportEvent::ConnectionClosed {
                        remote_addr,
                        transport_type: TransportType::Tcp,
                        ..
                    }
                    if remote_addr == server_addr
                ) {
                    saw_closed = true;
                    break;
                }
            }
        }
        assert!(saw_closed, "client never observed ConnectionClosed");

        client.close().await.unwrap();
    }
}
