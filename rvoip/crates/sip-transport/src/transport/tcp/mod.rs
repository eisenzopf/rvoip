mod connection;
mod listener;
pub mod pool;

pub use connection::TcpConnection;
pub use listener::TcpListener;
pub use pool::{ConnectionPool, PoolConfig};

use std::fmt;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::collections::HashMap;
use tokio::sync::{mpsc, Mutex};
use tracing::{debug, error, info, trace, warn};

use rvoip_sip_core::Message;
use crate::error::{Error, Result};
use crate::transport::{Transport, TransportEvent};

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
                                let _ = inner.events_tx.send(TransportEvent::Error {
                                    error: format!("Connection setup error: {}", e),
                                }).await;
                                continue;
                            }
                        };
                        
                        // Handle the connection
                        transport.clone().spawn_connection_handler(connection);
                    },
                    Err(e) => {
                        if inner.closed.load(Ordering::Relaxed) {
                            break;
                        }
                        
                        error!("Error accepting TCP connection: {}", e);
                        let _ = inner.events_tx.send(TransportEvent::Error {
                            error: format!("Accept error: {}", e),
                        }).await;
                        
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
    
    /// Spawns a handler for a new connection
    fn spawn_connection_handler(&self, connection: TcpConnection) {
        let transport = self.clone();
        let peer_addr = connection.peer_addr();
        
        tokio::spawn(async move {
            let inner = &transport.inner;
            let events_tx = inner.events_tx.clone();
            
            // Start reading messages from the connection
            while !inner.closed.load(Ordering::Relaxed) {
                match connection.receive_message().await {
                    Ok(Some(message)) => {
                        debug!("Received SIP message from {}", peer_addr);
                        
                        let local_addr = match connection.local_addr() {
                            Ok(addr) => addr,
                            Err(e) => {
                                error!("Failed to get local address: {}", e);
                                break;
                            }
                        };
                        
                        // Send the message to the channel
                        let event = TransportEvent::MessageReceived {
                            message,
                            source: peer_addr,
                            destination: local_addr,
                        };
                        
                        if let Err(e) = events_tx.send(event).await {
                            error!("Error sending event: {}", e);
                            break;
                        }
                    },
                    Ok(None) => {
                        // Connection closed gracefully
                        info!("Connection from {} closed gracefully", peer_addr);
                        break;
                    },
                    Err(e) => {
                        if inner.closed.load(Ordering::Relaxed) {
                            break;
                        }
                        
                        error!("Error reading from connection {}: {}", peer_addr, e);
                        let _ = events_tx.send(TransportEvent::Error {
                            error: format!("Connection error from {}: {}", peer_addr, e),
                        }).await;
                        break;
                    }
                }
            }
            
            // Remove the connection from any pools or maps
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
        
        // Add the connection to the pool and return it
        let connection_arc = Arc::new(connection);
        self.inner.connection_pool.add_connection(addr, connection_arc.clone()).await;
        
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
        
        debug!("Sending {} message to {}", 
            if let Message::Request(ref req) = message { 
                format!("{}", req.method) 
            } else { 
                "response".to_string() 
            }, 
            destination);
        
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
    use tokio::time::Duration;
    use rvoip_sip_core::{Method, Request};
    use rvoip_sip_core::builder::SimpleRequestBuilder;
    
    #[tokio::test]
    async fn test_tcp_transport_bind() {
        let config = PoolConfig {
            max_connections: 10,
            idle_timeout: Duration::from_secs(10),
        };
        
        let (transport, _rx) = TcpTransport::bind(
            "127.0.0.1:0".parse().unwrap(),
            Some(10),
            Some(config)
        ).await.unwrap();
        
        let addr = transport.local_addr().unwrap();
        assert!(addr.port() > 0);
        
        transport.close().await.unwrap();
        assert!(transport.is_closed());
    }
    
    #[tokio::test]
    async fn test_tcp_transport_send_receive() {
        // Start a server transport
        let (server_transport, mut server_rx) = TcpTransport::bind(
            "127.0.0.1:0".parse().unwrap(),
            Some(10),
            None
        ).await.unwrap();
        
        let server_addr = server_transport.local_addr().unwrap();
        
        // Start a client transport
        let (client_transport, _client_rx) = TcpTransport::bind(
            "127.0.0.1:0".parse().unwrap(), 
            Some(10),
            None
        ).await.unwrap();
        
        // Create a test SIP message
        let request = SimpleRequestBuilder::new(Method::Register, "sip:example.com")
            .unwrap()
            .from("alice", "sip:alice@example.com", Some("tag1"))
            .to("bob", "sip:bob@example.com", None)
            .call_id("call1@example.com")
            .cseq(1)
            .build();
        
        // Send the message
        client_transport.send_message(request.into(), server_addr).await.unwrap();
        
        // Receive the message
        let event = tokio::time::timeout(Duration::from_secs(5), server_rx.recv())
            .await
            .unwrap()
            .unwrap();
        
        match event {
            TransportEvent::MessageReceived { message, source, destination } => {
                assert_eq!(destination, server_addr);
                assert!(message.is_request());
                if let Message::Request(req) = message {
                    assert_eq!(req.method(), Method::Register);
                } else {
                    panic!("Expected a request");
                }
            },
            _ => panic!("Expected MessageReceived event"),
        }
        
        // Clean up
        client_transport.close().await.unwrap();
        server_transport.close().await.unwrap();
    }
} 