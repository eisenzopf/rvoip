mod connection;
mod listener;

pub use connection::WebSocketConnection;
pub use listener::WebSocketListener;

use std::fmt;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use std::collections::HashMap;
use tokio::sync::{mpsc, Mutex};
use futures_util::StreamExt;
use tracing::{debug, error, info, trace, warn};

use rvoip_sip_core::Message;
use crate::error::{Error, Result};
use crate::transport::{Transport, TransportEvent};

// SIP WebSocket subprotocol names as per RFC 7118
pub(crate) const SIP_WS_SUBPROTOCOL: &str = "sip";
pub(crate) const SIP_WSS_SUBPROTOCOL: &str = "sips";

// Default channel capacity
const DEFAULT_CHANNEL_CAPACITY: usize = 100;

/// WebSocket transport for SIP messages
#[derive(Clone)]
pub struct WebSocketTransport {
    inner: Arc<WebSocketTransportInner>,
}

struct WebSocketTransportInner {
    listener: Arc<WebSocketListener>,
    connections: Mutex<HashMap<SocketAddr, Arc<WebSocketConnection>>>,
    closed: AtomicBool,
    events_tx: mpsc::Sender<TransportEvent>,
}

impl WebSocketTransport {
    /// Creates a new WebSocket transport bound to the specified address
    pub async fn bind(
        addr: SocketAddr,
        secure: bool,
        cert_path: Option<&str>,
        key_path: Option<&str>,
        channel_capacity: Option<usize>,
    ) -> Result<(Self, mpsc::Receiver<TransportEvent>)> {
        // Create the event channel
        let capacity = channel_capacity.unwrap_or(DEFAULT_CHANNEL_CAPACITY);
        let (events_tx, events_rx) = mpsc::channel(capacity);
        
        // Create the WebSocket listener
        let listener = WebSocketListener::bind(addr, secure, cert_path, key_path).await?;
        let local_addr = listener.local_addr()?;
        
        info!("SIP WebSocket transport bound to {} ({})", 
              local_addr, if secure { "wss" } else { "ws" });
        
        // Create the transport
        let transport = WebSocketTransport {
            inner: Arc::new(WebSocketTransportInner {
                listener: Arc::new(listener),
                connections: Mutex::new(HashMap::new()),
                closed: AtomicBool::new(false),
                events_tx: events_tx.clone(),
            }),
        };

        // Start the accept loop to accept incoming connections
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
                    Ok((connection, mut reader)) => {
                        let peer_addr = connection.peer_addr();
                        debug!("Accepted WebSocket connection from {}", peer_addr);
                        
                        // Store the connection
                        let connection_arc = Arc::new(connection);
                        {
                            let mut connections = inner.connections.lock().await;
                            connections.insert(peer_addr, connection_arc.clone());
                        }
                        
                        // Handle the connection
                        transport.clone().spawn_connection_reader(connection_arc, reader);
                    },
                    Err(e) => {
                        if inner.closed.load(Ordering::Relaxed) {
                            break;
                        }
                        
                        error!("Error accepting WebSocket connection: {}", e);
                        let _ = inner.events_tx.send(TransportEvent::Error {
                            error: format!("Accept error: {}", e),
                        }).await;
                        
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
        mut reader: futures_util::stream::SplitStream<tokio_tungstenite::WebSocketStream<tokio_tungstenite::MaybeTlsStream<tokio::net::TcpStream>>>,
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
                        error!("Error reading from WebSocket connection {}: {}", peer_addr, e);
                        
                        let _ = inner.events_tx.send(TransportEvent::Error {
                            error: format!("WebSocket read error from {}: {}", peer_addr, e),
                        }).await;
                        
                        break;
                    },
                    None => {
                        // End of stream
                        debug!("WebSocket connection from {} closed by peer", peer_addr);
                        break;
                    }
                };
                
                // Process the WebSocket message
                match connection.process_ws_message(ws_message) {
                    Ok(Some(sip_message)) => {
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
                        };
                        
                        if let Err(e) = inner.events_tx.send(event).await {
                            error!("Error sending event: {}", e);
                            break;
                        }
                    },
                    Ok(None) => {
                        // Control message like ping/pong/close, already handled
                        continue;
                    },
                    Err(e) => {
                        warn!("Error processing WebSocket message from {}: {}", peer_addr, e);
                        
                        let _ = inner.events_tx.send(TransportEvent::Error {
                            error: format!("WebSocket message processing error: {}", e),
                        }).await;
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
    
    /// Connect to a remote WebSocket server
    #[cfg(feature = "ws")]
    async fn connect_to(&self, addr: SocketAddr) -> Result<Arc<WebSocketConnection>> {
        // Check if we already have an open connection
        {
            let connections = self.inner.connections.lock().await;
            if let Some(conn) = connections.get(&addr) {
                if !conn.is_closed() {
                    return Ok(conn.clone());
                }
            }
        }
        
        // Not implemented yet - we'll need to implement client-side WebSocket connection
        Err(Error::NotImplemented("WebSocket client connections not yet implemented".into()))
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
        
        debug!("Sending {} message to {}", 
            if let Message::Request(ref req) = message { 
                format!("{}", req.method) 
            } else { 
                "response".to_string() 
            }, 
            destination);
        
        #[cfg(feature = "ws")]
        {
            // Get or create a connection to the destination
            let connection = self.connect_to(destination).await?;
            
            // Send the message
            connection.send_message(&message).await
        }
        
        #[cfg(not(feature = "ws"))]
        Err(Error::NotImplemented("WebSocket transport not implemented".into()))
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
    use tokio::time::Duration;
    use rvoip_sip_core::{Method, Request};
    use rvoip_sip_core::builder::SimpleRequestBuilder;
    
    #[cfg(feature = "ws")]
    #[tokio::test]
    async fn test_websocket_transport_bind() {
        let result = WebSocketTransport::bind(
            "127.0.0.1:0".parse().unwrap(),
            false,
            None,
            None,
            None,
        ).await;
        
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
    async fn test_websocket_transport_secure_bind() {
        // Test binding with secure WebSocket (WSS)
        let result = WebSocketTransport::bind(
            "127.0.0.1:0".parse().unwrap(),
            true,
            Some("cert.pem"),
            Some("key.pem"),
            None,
        ).await;
        
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
    async fn test_websocket_transport_not_implemented_send() {
        // Test sending to a non-existent connection (this should fail with NotImplemented)
        let (transport, _rx) = WebSocketTransport::bind(
            "127.0.0.1:0".parse().unwrap(),
            false,
            None,
            None,
            None,
        ).await.unwrap();
        
        // Create a test SIP message
        let request = SimpleRequestBuilder::new(Method::Register, "sip:example.com")
            .unwrap()
            .from("alice", "sip:alice@example.com", Some("tag1"))
            .to("bob", "sip:bob@example.com", None)
            .call_id("call1@example.com")
            .cseq(1)
            .build();
        
        // Try to send the message (should fail since client connections aren't implemented yet)
        let result = transport.send_message(request.into(), "127.0.0.1:5060".parse().unwrap()).await;
        assert!(result.is_err());
        
        if let Err(e) = result {
            assert!(matches!(e, Error::NotImplemented(_)));
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
        ).await.unwrap();
        
        // Close the transport - this should send a Closed event
        transport.close().await.unwrap();
        
        // Wait for the closed event
        let event = tokio::time::timeout(Duration::from_secs(1), rx.recv()).await.unwrap();
        
        // Verify the event
        assert!(matches!(event, Some(TransportEvent::Closed)));
    }
    
    #[cfg(feature = "ws")]
    #[tokio::test]
    async fn test_websocket_transport_debug_fmt() {
        // Test the Debug implementation
        let (transport, _rx) = WebSocketTransport::bind(
            "127.0.0.1:0".parse().unwrap(),
            false,
            None,
            None,
            None,
        ).await.unwrap();
        
        let debug_str = format!("{:?}", transport);
        assert!(debug_str.starts_with("WebSocketTransport(127.0.0.1:"));
        
        transport.close().await.unwrap();
    }
    
    // Tests for client connection support would go here once implemented
} 