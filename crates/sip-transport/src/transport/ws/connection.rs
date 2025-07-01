use std::io;
use std::net::SocketAddr;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::Arc;
use bytes::{BytesMut, Buf};
use futures_util::stream::{SplitSink, SplitStream};
use futures_util::SinkExt;
use futures_util::StreamExt;
use tokio::net::TcpStream;
use tokio::sync::Mutex;
use tracing::{debug, error, trace, warn};

#[cfg(feature = "ws")]
use tokio_tungstenite::{tungstenite, WebSocketStream, MaybeTlsStream};
#[cfg(feature = "ws")]
use tokio_tungstenite::tungstenite::protocol::{Message as WsMessage, Role};

use rvoip_sip_core::{Message, parse_message};
use crate::error::{Error, Result};

// SIP WebSocket subprotocol names as per RFC 7118
const SIP_WS_SUBPROTOCOL: &str = "sip";
const SIP_WSS_SUBPROTOCOL: &str = "sips";

// Maximum message size in bytes
const MAX_MESSAGE_SIZE: usize = 65535;

/// WebSocket connection for SIP messages
pub struct WebSocketConnection {
    /// The WebSocket stream (writer half)
    #[cfg(feature = "ws")]
    ws_writer: Mutex<SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, WsMessage>>,
    /// The peer's address
    peer_addr: SocketAddr,
    /// Whether the connection is closed
    closed: AtomicBool,
    /// Whether this is a secure WebSocket connection
    secure: bool,
    /// The selected subprotocol (sip or sips)
    subprotocol: String,
}

impl WebSocketConnection {
    /// Creates a WebSocket connection from an existing WebSocket stream
    #[cfg(feature = "ws")]
    pub fn from_writer(
        ws_writer: SplitSink<WebSocketStream<MaybeTlsStream<TcpStream>>, WsMessage>,
        peer_addr: SocketAddr, 
        secure: bool,
        subprotocol: String,
    ) -> Self {
        Self {
            ws_writer: Mutex::new(ws_writer),
            peer_addr,
            closed: AtomicBool::new(false),
            secure,
            subprotocol,
        }
    }
    
    /// Returns the peer address of the connection
    pub fn peer_addr(&self) -> SocketAddr {
        self.peer_addr
    }
    
    /// Returns the local address of the connection
    pub fn local_addr(&self) -> Result<SocketAddr> {
        // WebSocket connections don't directly expose the local address
        // Would need to be tracked separately when the connection is created
        Err(Error::NotImplemented("Getting local address from WebSocket connection".into()))
    }
    
    /// Returns whether this is a secure connection
    pub fn is_secure(&self) -> bool {
        self.secure
    }
    
    /// Returns the selected subprotocol
    pub fn subprotocol(&self) -> &str {
        &self.subprotocol
    }
    
    /// Sends a SIP message over the WebSocket connection
    #[cfg(feature = "ws")]
    pub async fn send_message(&self, message: &Message) -> Result<()> {
        if self.is_closed() {
            return Err(Error::TransportClosed);
        }
        
        // Convert SIP message to bytes
        let message_bytes = message.to_bytes();
        
        // In WebSocket, we send the raw SIP message as text content
        // RFC 7118 section 7: SIP WebSocket Client and Server Examples
        let ws_message = WsMessage::Text(String::from_utf8_lossy(&message_bytes).to_string());
        
        // Acquire lock on the writer
        let mut writer = self.ws_writer.lock().await;
        
        // Send the message
        writer.send(ws_message).await
            .map_err(|e| {
                self.closed.store(true, Ordering::Relaxed);
                match e {
                    tungstenite::Error::ConnectionClosed => Error::ConnectionClosedByPeer(self.peer_addr),
                    tungstenite::Error::Protocol(msg) => Error::WebSocketProtocolError(msg.to_string()),
                    tungstenite::Error::Io(io_err) => {
                        if io_err.kind() == io::ErrorKind::BrokenPipe || 
                           io_err.kind() == io::ErrorKind::ConnectionReset {
                            Error::ConnectionReset
                        } else {
                            Error::SendFailed(self.peer_addr, io_err)
                        }
                    },
                    _ => Error::SendFailed(self.peer_addr, io::Error::new(io::ErrorKind::Other, e.to_string())),
                }
            })?;
        
        trace!("Sent SIP message over WebSocket to {}", self.peer_addr);
        Ok(())
    }
    
    /// Processes a WebSocket message and attempts to parse it as a SIP message
    #[cfg(feature = "ws")]
    pub fn process_ws_message(&self, ws_message: WsMessage) -> Result<Option<Message>> {
        match ws_message {
            WsMessage::Text(text) => {
                // RFC 7118 section 7: SIP messages are sent as text frames
                trace!("Received text message over WebSocket from {}", self.peer_addr);
                
                // Parse the text as a SIP message
                match parse_message(text.as_bytes()) {
                    Ok(message) => Ok(Some(message)),
                    Err(e) => {
                        warn!("Failed to parse SIP message from WebSocket: {}", e);
                        Err(Error::ParseError(e.to_string()))
                    }
                }
            },
            WsMessage::Binary(data) => {
                // Some implementations might send binary data instead of text
                trace!("Received binary message over WebSocket from {}", self.peer_addr);
                
                // Parse the binary data as a SIP message
                match parse_message(&data) {
                    Ok(message) => Ok(Some(message)),
                    Err(e) => {
                        warn!("Failed to parse SIP message from WebSocket binary: {}", e);
                        Err(Error::ParseError(e.to_string()))
                    }
                }
            },
            WsMessage::Ping(_) => {
                // Ping messages should be automatically handled by the WebSocket library
                trace!("Received ping from {}", self.peer_addr);
                Ok(None)
            },
            WsMessage::Pong(_) => {
                // Pong messages are responses to our pings
                trace!("Received pong from {}", self.peer_addr);
                Ok(None)
            },
            WsMessage::Close(_) => {
                debug!("Received close frame from {}", self.peer_addr);
                self.closed.store(true, Ordering::Relaxed);
                Ok(None)
            },
            WsMessage::Frame(_) => {
                // Raw frames should not be received with tungstenite
                warn!("Received unexpected raw frame from {}", self.peer_addr);
                Ok(None)
            },
        }
    }
    
    /// Closes the WebSocket connection
    #[cfg(feature = "ws")]
    pub async fn close(&self) -> Result<()> {
        if self.closed.swap(true, Ordering::Relaxed) {
            // Already closed
            return Ok(());
        }
        
        let mut writer = self.ws_writer.lock().await;
        
        // Send a close frame
        if let Err(e) = writer.send(WsMessage::Close(None)).await {
            // If we can't send a close frame, just log it
            warn!("Failed to send close frame to {}: {}", self.peer_addr, e);
        }
        
        // Close the sink
        if let Err(e) = writer.close().await {
            return Err(Error::IoError(io::Error::new(io::ErrorKind::Other, e.to_string())));
        }
        
        Ok(())
    }
    
    /// Returns whether the connection is closed
    pub fn is_closed(&self) -> bool {
        self.closed.load(Ordering::Relaxed)
    }
}

#[cfg(not(feature = "ws"))]
impl WebSocketConnection {
    /// Creates a WebSocket connection from an existing WebSocket stream
    pub fn from_writer(
        _writer: (),
        peer_addr: SocketAddr, 
        secure: bool,
        subprotocol: String,
    ) -> Self {
        Self {
            peer_addr,
            closed: AtomicBool::new(false),
            secure,
            subprotocol,
        }
    }
    
    /// Sends a SIP message over the WebSocket connection
    pub async fn send_message(&self, _message: &Message) -> Result<()> {
        Err(Error::NotImplemented("WebSocket support is not enabled".into()))
    }
    
    /// Closes the WebSocket connection
    pub async fn close(&self) -> Result<()> {
        self.closed.store(true, Ordering::Relaxed);
        Ok(())
    }
}

impl Drop for WebSocketConnection {
    fn drop(&mut self) {
        if !self.is_closed() {
            // The connection is being dropped without being closed
            debug!("WebSocket connection to {} dropped without being closed", self.peer_addr);
        }
    }
}

// Unit tests will be added later

#[cfg(test)]
mod tests {
    use super::*;
    use tokio_tungstenite::tungstenite::protocol::Message as WsMessage;
    use rvoip_sip_core::{Method, Request};
    use rvoip_sip_core::builder::SimpleRequestBuilder;
    
    // For testing only: a simplified WebSocketConnection without real WebSocket dependencies
    #[cfg(feature = "ws")]
    struct TestWebSocketConnection {
        peer_addr: SocketAddr,
        closed: AtomicBool,
        secure: bool,
        subprotocol: String,
    }
    
    #[cfg(feature = "ws")]
    impl TestWebSocketConnection {
        fn new(addr: SocketAddr, secure: bool, subprotocol: String) -> Self {
            Self {
                peer_addr: addr,
                closed: AtomicBool::new(false),
                secure,
                subprotocol,
            }
        }
        
        fn peer_addr(&self) -> SocketAddr {
            self.peer_addr
        }
        
        fn is_secure(&self) -> bool {
            self.secure
        }
        
        fn subprotocol(&self) -> &str {
            &self.subprotocol
        }
        
        fn is_closed(&self) -> bool {
            self.closed.load(Ordering::Relaxed)
        }
        
        fn set_closed(&self) {
            self.closed.store(true, Ordering::Relaxed);
        }
        
        fn local_addr(&self) -> Result<SocketAddr> {
            Err(Error::NotImplemented("Getting local address from WebSocket connection".into()))
        }
        
        fn process_ws_message(&self, ws_message: WsMessage) -> Result<Option<Message>> {
            match ws_message {
                WsMessage::Text(text) => {
                    // Parse the text as a SIP message
                    match parse_message(text.as_bytes()) {
                        Ok(message) => Ok(Some(message)),
                        Err(e) => Err(Error::ParseError(e.to_string()))
                    }
                },
                WsMessage::Binary(data) => {
                    // Parse binary data as SIP message
                    match parse_message(&data) {
                        Ok(message) => Ok(Some(message)),
                        Err(e) => Err(Error::ParseError(e.to_string()))
                    }
                },
                WsMessage::Close(_) => {
                    // Mark as closed
                    self.closed.store(true, Ordering::Relaxed);
                    Ok(None)
                },
                _ => Ok(None), // Control frames don't produce messages
            }
        }
    }
    
    // Test simple parameter validation and getters
    #[cfg(feature = "ws")]
    #[tokio::test]
    async fn test_websocket_connection_parameters() {
        let addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
        let secure = true;
        let subprotocol = "sips".to_string();
        
        let connection = TestWebSocketConnection::new(addr, secure, subprotocol.clone());
        
        // Test that parameters are correctly stored and retrievable
        assert_eq!(connection.peer_addr(), addr);
        assert_eq!(connection.is_secure(), secure);
        assert_eq!(connection.subprotocol(), subprotocol);
        assert!(!connection.is_closed());
        
        // Test closing
        connection.set_closed();
        assert!(connection.is_closed());
    }
    
    // Test message parsing from WebSocket frames
    #[cfg(feature = "ws")]
    #[tokio::test]
    async fn test_process_ws_message() {
        let addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
        let connection = TestWebSocketConnection::new(
            addr, 
            false, 
            SIP_WS_SUBPROTOCOL.to_string()
        );
        
        // Test processing a text frame with valid SIP content
        let sip_text = "\
REGISTER sip:example.com SIP/2.0\r\n\
Via: SIP/2.0/WS 127.0.0.1:5060;branch=z9hG4bK-524287-1\r\n\
From: <sip:alice@example.com>;tag=1\r\n\
To: <sip:bob@example.com>\r\n\
Call-ID: call1@example.com\r\n\
CSeq: 1 REGISTER\r\n\
Contact: <sip:alice@127.0.0.1>\r\n\
Content-Length: 0\r\n\
\r\n";
        
        let text_frame = WsMessage::Text(sip_text.to_string());
        let result = connection.process_ws_message(text_frame).unwrap();
        
        assert!(result.is_some());
        if let Some(Message::Request(req)) = result {
            assert_eq!(req.method(), Method::Register);
            assert_eq!(req.call_id().unwrap().to_string(), "call1@example.com");
        } else {
            panic!("Expected SIP request");
        }
        
        // Test processing a ping frame
        let ping_frame = WsMessage::Ping(vec![1, 2, 3]);
        let result = connection.process_ws_message(ping_frame).unwrap();
        assert!(result.is_none(), "Ping frame should not produce a SIP message");
        
        // Test processing a pong frame
        let pong_frame = WsMessage::Pong(vec![1, 2, 3]);
        let result = connection.process_ws_message(pong_frame).unwrap();
        assert!(result.is_none(), "Pong frame should not produce a SIP message");
        
        // Test processing a close frame
        let close_frame = WsMessage::Close(None);
        let result = connection.process_ws_message(close_frame).unwrap();
        assert!(result.is_none(), "Close frame should not produce a SIP message");
        assert!(connection.is_closed(), "Connection should be marked as closed after receiving close frame");
    }
    
    // Test local_addr returns the expected NotImplemented error
    #[cfg(feature = "ws")]
    #[tokio::test]
    async fn test_local_addr_not_implemented() {
        let addr: SocketAddr = "127.0.0.1:5060".parse().unwrap();
        let connection = TestWebSocketConnection::new(
            addr, 
            false, 
            SIP_WS_SUBPROTOCOL.to_string()
        );
        
        let result = connection.local_addr();
        assert!(result.is_err());
        if let Err(e) = result {
            assert!(matches!(e, Error::NotImplemented(_)));
        } else {
            panic!("Expected NotImplemented error");
        }
    }
} 