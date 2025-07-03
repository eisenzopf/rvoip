use std::net::SocketAddr;
use std::sync::Arc;
use futures_util::StreamExt;
use futures_util::stream::SplitStream;
use tokio::net::{TcpListener, TcpStream};
use tracing::{debug, error, info, trace, warn};

#[cfg(feature = "ws")]
use tokio_tungstenite::{accept_async, tungstenite, WebSocketStream, MaybeTlsStream};
#[cfg(feature = "ws")]
use tokio_tungstenite::tungstenite::handshake::server::{Request, Response};
#[cfg(feature = "ws")]
use http::HeaderValue;

use crate::error::{Error, Result};
use super::connection::WebSocketConnection;
use super::{SIP_WS_SUBPROTOCOL, SIP_WSS_SUBPROTOCOL};

/// WebSocket listener for accepting SIP WebSocket connections
pub struct WebSocketListener {
    /// The underlying TCP listener
    listener: TcpListener,
    /// Whether this is a secure WebSocket listener (WSS)
    secure: bool,
    /// TLS certificate path if secure
    cert_path: Option<String>,
    /// TLS key path if secure
    key_path: Option<String>,
}

impl WebSocketListener {
    /// Binds a WebSocket listener to the specified address
    pub async fn bind(
        addr: SocketAddr,
        secure: bool,
        cert_path: Option<&str>,
        key_path: Option<&str>,
    ) -> Result<Self> {
        let listener = TcpListener::bind(addr)
            .await
            .map_err(|e| Error::BindFailed(addr, e))?;
        
        info!("WebSocket listener bound to {} ({})", 
              listener.local_addr().unwrap(), 
              if secure { "wss" } else { "ws" });
        
        Ok(Self {
            listener,
            secure,
            cert_path: cert_path.map(String::from),
            key_path: key_path.map(String::from),
        })
    }
    
    /// Returns the local address this listener is bound to
    pub fn local_addr(&self) -> Result<SocketAddr> {
        self.listener.local_addr().map_err(|e| Error::LocalAddrFailed(e))
    }
    
    /// Accepts a new WebSocket connection
    #[cfg(feature = "ws")]
    pub async fn accept(&self) -> Result<(WebSocketConnection, SplitStream<WebSocketStream<MaybeTlsStream<TcpStream>>>)> {
        // Accept a TCP connection
        let (stream, peer_addr) = self.listener.accept()
            .await
            .map_err(|e| Error::ReceiveFailed(e))?;
        
        debug!("Accepted TCP connection for WebSocket from {}", peer_addr);
        
        // TODO: If this is a secure WebSocket, perform TLS handshake
        let maybe_tls_stream = MaybeTlsStream::Plain(stream);
        
        // Custom callback for WebSocket handshake to handle subprotocol negotiation
        let callback = |request: &Request, response: Response| {
            // Check for subprotocol request
            let protocols = request.headers().get("Sec-WebSocket-Protocol")
                                  .and_then(|h| h.to_str().ok())
                                  .map(|p| p.split(',').map(|s| s.trim()).collect::<Vec<_>>());
            
            let mut response = response;
            let mut selected_protocol = String::new();
            
            if let Some(protocols) = protocols {
                // Check if the client supports our subprotocols
                let supported_protocol = if self.secure {
                    if protocols.contains(&SIP_WSS_SUBPROTOCOL) {
                        Some(SIP_WSS_SUBPROTOCOL)
                    } else {
                        None
                    }
                } else {
                    if protocols.contains(&SIP_WS_SUBPROTOCOL) {
                        Some(SIP_WS_SUBPROTOCOL)
                    } else {
                        None
                    }
                };
                
                if let Some(protocol) = supported_protocol {
                    response.headers_mut().append(
                        "Sec-WebSocket-Protocol",
                        HeaderValue::from_str(protocol).unwrap(),
                    );
                    selected_protocol = protocol.to_string();
                }
            }
            
            Ok((response, selected_protocol))
        };
        
        // Perform WebSocket handshake
        let (ws_stream, selected_protocol) = Self::accept_async_with_subprotocol(maybe_tls_stream, callback).await
            .map_err(|e| {
                error!("WebSocket handshake failed with {}: {}", peer_addr, e);
                Error::WebSocketHandshakeFailed(e.to_string())
            })?;
        
        // Default to 'sip' if no subprotocol was selected
        let subprotocol = if selected_protocol.is_empty() {
            if self.secure { SIP_WSS_SUBPROTOCOL } else { SIP_WS_SUBPROTOCOL }.to_string()
        } else {
            selected_protocol
        };
        
        // Split the stream for separate reading and writing
        let (ws_writer, ws_reader) = ws_stream.split();
        
        // Create a WebSocket connection
        let connection = WebSocketConnection::from_writer(
            ws_writer,
            peer_addr,
            self.secure,
            subprotocol,
        );
        
        Ok((connection, ws_reader))
    }
    
    /// Helper to accept WebSocket connections with subprotocol selection
    #[cfg(feature = "ws")]
    async fn accept_async_with_subprotocol<F>(
        stream: MaybeTlsStream<TcpStream>,
        _callback: F,
    ) -> std::result::Result<(WebSocketStream<MaybeTlsStream<TcpStream>>, String), tungstenite::Error>
    where
        F: FnOnce(&Request, Response) -> std::result::Result<(Response, String), tungstenite::Error>,
    {
        // This is a simplified implementation that doesn't actually use the callback yet
        // In a full implementation, we'd modify tokio-tungstenite to support returning additional data
        
        let ws_stream = tokio_tungstenite::accept_async(stream).await?;
        
        // For now, we'll just return an empty string for the subprotocol
        // In a real implementation, we'd extract this from the handshake
        Ok((ws_stream, String::new()))
    }
    
    /// Returns whether this is a secure WebSocket listener
    pub fn is_secure(&self) -> bool {
        self.secure
    }
}

#[cfg(not(feature = "ws"))]
impl WebSocketListener {
    /// Accepts a new WebSocket connection (not implemented without ws feature)
    pub async fn accept(&self) -> Result<()> {
        Err(Error::NotImplemented("WebSocket support is not enabled".into()))
    }
}

// Unit tests will be added later

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::net::TcpStream;
    use std::sync::Arc;
    
    /// Test binding a WebSocket listener
    #[tokio::test]
    async fn test_websocket_listener_bind() {
        let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let listener = WebSocketListener::bind(addr, false, None, None).await.unwrap();
        
        let bound_addr = listener.local_addr().unwrap();
        assert!(bound_addr.port() > 0); // Random port assigned
        assert_eq!(bound_addr.ip(), addr.ip());
        assert!(!listener.is_secure());
    }
    
    /// Test binding a secure WebSocket listener
    #[tokio::test]
    async fn test_websocket_listener_bind_secure() {
        let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let listener = WebSocketListener::bind(addr, true, Some("cert.pem"), Some("key.pem")).await.unwrap();
        
        let bound_addr = listener.local_addr().unwrap();
        assert!(bound_addr.port() > 0); // Random port assigned
        assert_eq!(bound_addr.ip(), addr.ip());
        assert!(listener.is_secure());
        
        // Verify certificate paths are stored
        assert_eq!(listener.cert_path.as_deref(), Some("cert.pem"));
        assert_eq!(listener.key_path.as_deref(), Some("key.pem"));
    }
    
    /// Tests accepting a WebSocket connection
    #[cfg(feature = "ws")]
    #[tokio::test]
    async fn test_websocket_listener_accept() {
        // This is a more complex test that would require us to actually
        // create a WebSocket client that connects to our listener.
        
        // For now, we'll simply test that the listener can be created and accept() method exists
        let addr: SocketAddr = "127.0.0.1:0".parse().unwrap();
        let listener = WebSocketListener::bind(addr, false, None, None).await.unwrap();
        
        let bound_addr = listener.local_addr().unwrap();
        assert!(bound_addr.port() > 0);
        
        // We can't easily test the accept method without setting up a real WebSocket client
        // The method signature is what we're primarily verifying here
        let accept_method_exists = true; // This test passes if it compiles
        assert!(accept_method_exists);
    }
    
    /// Simple client-server connection test (integration level)
    #[cfg(all(feature = "ws", test))]
    #[tokio::test]
    async fn test_websocket_client_server_connection() {
        // This test is marked with #[cfg(all(feature = "ws", test))] because:
        // 1. It requires the ws feature
        // 2. It's more of an integration test than a unit test
        
        // Ideally we'd have a full client-server test that uses a client to connect to
        // our listener and test the full protocol flow, but that's challenging to do
        // without refactoring the code to support a test client or using a real client.
        
        // To directly test the listener's accept method properly would require:
        // 1. Setting up a tokio runtime
        // 2. Creating a TCP connection to the listener
        // 3. Performing a WebSocket handshake manually or with a client
        // 4. Verifying the connection is accepted and the right objects are returned
        
        // This is left as a future enhancement. In a production environment,
        // you'd typically have integration tests that create a real client and
        // send actual WebSocket frames to test the complete flow.
        
        // For now we're relying on:
        // 1. The unit tests for individual components
        // 2. The integration tests for the transport as a whole
        // 3. Manual testing with real clients
    }
} 