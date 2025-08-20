use std::fmt;
use std::net::SocketAddr;
use std::sync::Arc;

use rvoip_sip_core::Message;
use crate::error::Result;

pub mod udp;
pub mod tcp;
pub mod tls;
pub mod ws;

pub use udp::UdpTransport;
pub use tcp::TcpTransport;
pub use tls::TlsTransport;
pub use ws::WebSocketTransport;

/// Represents the transport type/protocol
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum TransportType {
    Udp,
    Tcp,
    Tls,
    Ws,
    Wss,
}

impl fmt::Display for TransportType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            TransportType::Udp => write!(f, "UDP"),
            TransportType::Tcp => write!(f, "TCP"),
            TransportType::Tls => write!(f, "TLS"),
            TransportType::Ws => write!(f, "WS"),
            TransportType::Wss => write!(f, "WSS"),
        }
    }
}

/// Events emitted by a transport
#[derive(Debug, Clone)]
pub enum TransportEvent {
    /// A SIP message was received
    MessageReceived {
        /// The SIP message
        message: Message,
        /// The remote address that sent the message
        source: SocketAddr,
        /// The local address that received the message
        destination: SocketAddr,
    },
    
    /// Error occurred in the transport
    Error {
        /// Error description
        error: String,
    },
    
    /// Transport has been closed
    Closed,
    
    // ========== GRACEFUL SHUTDOWN EVENTS ==========
    
    /// Shutdown request received from transaction layer
    ShutdownRequested,
    
    /// Transport is ready for shutdown
    ShutdownReady,
    
    /// Transport should shutdown now
    ShutdownNow,
    
    /// Transport shutdown complete
    ShutdownComplete,
}

/// Represents a transport layer for SIP messages.
///
/// This trait defines the common interface for all transport types (UDP, TCP, TLS, WebSocket).
#[async_trait::async_trait]
pub trait Transport: Send + Sync + fmt::Debug {
    /// Returns the local address this transport is bound to
    fn local_addr(&self) -> Result<SocketAddr>;
    
    /// Sends a SIP message to the specified destination
    async fn send_message(&self, message: Message, destination: SocketAddr) -> Result<()>;
    
    /// Closes the transport
    async fn close(&self) -> Result<()>;
    
    /// Checks if the transport is closed
    fn is_closed(&self) -> bool;

    /// Check if UDP transport is supported
    fn supports_udp(&self) -> bool {
        // Default implementation - UDP is commonly supported
        true
    }
    
    /// Check if TCP transport is supported
    fn supports_tcp(&self) -> bool {
        // Default implementation
        false
    }
    
    /// Check if TLS transport is supported
    fn supports_tls(&self) -> bool {
        // Default implementation
        false
    }
    
    /// Check if WebSocket transport is supported
    fn supports_ws(&self) -> bool {
        // Default implementation
        false
    }
    
    /// Check if Secure WebSocket transport is supported
    fn supports_wss(&self) -> bool {
        // Default implementation
        false
    }
    
    /// Check if a specific transport type is supported
    fn supports_transport(&self, transport_type: TransportType) -> bool {
        match transport_type {
            TransportType::Udp => self.supports_udp(),
            TransportType::Tcp => self.supports_tcp(),
            TransportType::Tls => self.supports_tls(),
            TransportType::Ws => self.supports_ws(),
            TransportType::Wss => self.supports_wss(),
        }
    }
    
    /// Get the default transport type
    fn default_transport_type(&self) -> TransportType {
        // Most implementations default to UDP
        TransportType::Udp
    }
    
    /// Check if a specific transport is currently connected
    fn is_transport_connected(&self, transport_type: TransportType) -> bool {
        // For UDP, always considered connected
        // For connection-oriented transports, this would check connection status
        if transport_type == TransportType::Udp {
            true
        } else {
            !self.is_closed()
        }
    }
    
    /// Get the number of active connections for a transport type
    fn get_connection_count(&self, transport_type: TransportType) -> usize {
        // Default implementation
        if self.is_closed() { 0 } else { 1 }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicBool, Ordering};
    use tokio::sync::mpsc;
    
    // Mock transport for testing the trait
    #[derive(Debug)]
    struct MockTransport {
        closed: Arc<AtomicBool>,
        local_addr: SocketAddr,
    }
    
    impl MockTransport {
        fn new(addr: &str) -> Self {
            Self {
                closed: Arc::new(AtomicBool::new(false)),
                local_addr: addr.parse().unwrap(),
            }
        }
    }
    
    #[async_trait::async_trait]
    impl Transport for MockTransport {
        fn local_addr(&self) -> Result<SocketAddr> {
            Ok(self.local_addr)
        }
        
        async fn send_message(&self, _message: Message, _destination: SocketAddr) -> Result<()> {
            if self.closed.load(Ordering::Relaxed) {
                return Err(crate::error::Error::TransportClosed);
            }
            Ok(())
        }
        
        async fn close(&self) -> Result<()> {
            self.closed.store(true, Ordering::Relaxed);
            Ok(())
        }
        
        fn is_closed(&self) -> bool {
            self.closed.load(Ordering::Relaxed)
        }
    }
    
    #[tokio::test]
    async fn test_mock_transport() {
        let transport = MockTransport::new("127.0.0.1:5060");
        assert_eq!(transport.local_addr().unwrap().to_string(), "127.0.0.1:5060");
        assert!(!transport.is_closed());
        
        transport.close().await.unwrap();
        assert!(transport.is_closed());
    }
} 