mod connection;
mod listener;
pub mod pool;

pub use connection::{ReceivedFrame, TcpConnection};
pub use listener::TcpListener;
pub use pool::{ConnectionPool, PoolConfig};

use bytes::Bytes;
use std::collections::HashMap;
use std::fmt;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use tokio::sync::{mpsc, Mutex};
use tracing::{debug, error, info, trace, warn};

use crate::error::{Error, Result};
use crate::transport::{Transport, TransportEvent, TransportType};
use rvoip_sip_core::Message;

// Default channel capacity
const DEFAULT_CHANNEL_CAPACITY: usize = 100;
// Default connection idle timeout in seconds
const DEFAULT_IDLE_TIMEOUT_SECS: u64 = 300; // 5 minutes

/// TCP transport for SIP messages with connection pooling
#[derive(Clone)]
pub struct TcpTransport {
    inner: Arc<TcpTransportInner>,
}

struct TcpTransportInner {
    listener: Arc<TcpListener>,
    connection_pool: ConnectionPool,
    closed: AtomicBool,
    events_tx: mpsc::Sender<TransportEvent>,
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
        let connection_pool = ConnectionPool::new(config);

        // Create the transport
        let transport = TcpTransport {
            inner: Arc::new(TcpTransportInner {
                listener: Arc::new(listener),
                connection_pool,
                closed: AtomicBool::new(false),
                events_tx: events_tx.clone(),
            }),
        };

        // Start the accept loop to accept incoming connections
        transport.spawn_accept_loop();

        Ok((transport, events_rx))
    }

    /// Spawns a task to accept incoming connections
    fn spawn_accept_loop(&self) {
        let transport = self.clone();

        tokio::spawn(async move {
            let inner = &transport.inner;
            let listener_clone = inner.listener.clone();

            while !inner.closed.load(Ordering::Relaxed) {
                // Accept a new connection
                match listener_clone.accept().await {
                    Ok((stream, peer_addr)) => {
                        debug!("Accepted TCP connection from {}", peer_addr);

                        // Create a connection object
                        let connection = match TcpConnection::from_stream(stream, peer_addr) {
                            Ok(conn) => conn,
                            Err(e) => {
                                error!("Failed to create connection from stream: {}", e);
                                let _ = inner
                                    .events_tx
                                    .send(TransportEvent::Error {
                                        error: format!("Connection setup error: {}", e),
                                    })
                                    .await;
                                continue;
                            }
                        };

                        // Publish the connection into the pool so that outbound
                        // writes (responses, and RFC 5626 CRLFCRLF keep-alives
                        // via `send_raw`) can find it, then spawn the unified
                        // reader.
                        let arc = Arc::new(connection);
                        inner
                            .connection_pool
                            .add_connection(peer_addr, arc.clone())
                            .await;
                        transport.clone().spawn_connection_handler(arc);
                    }
                    Err(e) => {
                        if inner.closed.load(Ordering::Relaxed) {
                            break;
                        }

                        error!("Error accepting TCP connection: {}", e);
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
            info!("TCP accept loop terminated");
            let _ = inner.events_tx.send(TransportEvent::Closed).await;
        });
    }

    /// Spawns a handler that drains frames (SIP messages and RFC 5626
    /// keep-alive frames) off a pooled connection until EOF / error.
    /// Used by both inbound-accepted and outbound-dialled connections
    /// so that both directions see keep-alive pongs and emit
    /// `ConnectionClosed` before pool eviction.
    fn spawn_connection_handler(&self, connection: Arc<TcpConnection>) {
        let transport = self.clone();
        let peer_addr = connection.peer_addr();

        tokio::spawn(async move {
            let inner = &transport.inner;
            let events_tx = inner.events_tx.clone();

            loop {
                if inner.closed.load(Ordering::Relaxed) {
                    break;
                }

                match connection.receive_frame().await {
                    Ok(Some(ReceivedFrame::Message(message))) => {
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
                        };

                        if let Err(e) = events_tx.send(event).await {
                            error!("Error sending event: {}", e);
                            break;
                        }
                    }
                    Ok(Some(ReceivedFrame::KeepAlivePong)) => {
                        let local_addr = connection.local_addr().unwrap_or(peer_addr);
                        let _ = events_tx
                            .send(TransportEvent::KeepAlivePongReceived {
                                source: peer_addr,
                                destination: local_addr,
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
                        if inner.closed.load(Ordering::Relaxed) {
                            break;
                        }

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
            let _ = events_tx
                .send(TransportEvent::ConnectionClosed {
                    remote_addr: peer_addr,
                    transport_type: TransportType::Tcp,
                })
                .await;

            inner.connection_pool.remove_connection(&peer_addr).await;
        });
    }

    /// Connects to a remote address and returns a connection
    async fn connect_to(&self, addr: SocketAddr) -> Result<Arc<TcpConnection>> {
        // Check if there's already a connection in the pool
        if let Some(conn) = self.inner.connection_pool.get_connection(&addr).await {
            trace!("Reusing existing connection to {}", addr);
            return Ok(conn);
        }

        // No existing connection, create a new one
        debug!("Creating new connection to {}", addr);
        let connection = TcpConnection::connect(addr).await?;

        // Add the connection to the pool and spawn a reader so inbound
        // responses (and RFC 5626 keep-alive frames) from the server
        // side are surfaced as events. Without this, outbound-initiated
        // TCP flows would silently fill their read buffers.
        let connection_arc = Arc::new(connection);
        self.inner
            .connection_pool
            .add_connection(addr, connection_arc.clone())
            .await;
        self.clone()
            .spawn_connection_handler(connection_arc.clone());

        Ok(connection_arc)
    }
}

#[async_trait::async_trait]
impl Transport for TcpTransport {
    fn local_addr(&self) -> Result<SocketAddr> {
        self.inner.listener.local_addr()
    }

    async fn send_message(&self, message: Message, destination: SocketAddr) -> Result<()> {
        if self.is_closed() {
            return Err(Error::TransportClosed);
        }

        debug!(
            "Sending {} message to {}",
            if let Message::Request(ref req) = message {
                format!("{}", req.method)
            } else {
                "response".to_string()
            },
            destination
        );

        // Get or create a connection to the destination
        let connection = self.connect_to(destination).await?;

        // Send the message
        connection.send_message(&message).await
    }

    async fn close(&self) -> Result<()> {
        // Set the closed flag to prevent new operations
        self.inner.closed.store(true, Ordering::Relaxed);

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

    async fn send_raw(&self, destination: SocketAddr, data: Bytes) -> Result<()> {
        if self.is_closed() {
            return Err(Error::TransportClosed);
        }

        // RFC 5626 keep-alive: only reuse an existing pooled connection
        // — never open a fresh TCP dial for a bare-bytes write. If the
        // flow is gone, the caller (typically a ping task in dialog-
        // core) terminates; a fresh flow is the upper layer's job.
        let Some(connection) = self
            .inner
            .connection_pool
            .get_connection(&destination)
            .await
        else {
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
        if let Ok(addr) = self.inner.listener.local_addr() {
            write!(f, "TcpTransport({})", addr)
        } else {
            write!(f, "TcpTransport(<e>)")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rvoip_sip_core::builder::SimpleRequestBuilder;
    use rvoip_sip_core::{Method, Request};
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
                source,
                destination,
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
                    TransportEvent::ConnectionClosed { remote_addr, transport_type: TransportType::Tcp }
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
