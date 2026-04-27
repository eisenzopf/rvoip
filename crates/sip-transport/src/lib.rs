//! SIP transport layer implementation for the rvoip stack
//!
//! This crate provides transport implementations for SIP messages, including
//! UDP, TCP, TLS, and WebSocket transports.

// Re-export modules from the transport directory
pub mod error;
pub mod events;
pub mod factory;
pub mod manager;
pub mod transport;

// Internal modules
#[cfg(test)]
mod tests;

// Re-export commonly used types and functions
pub use error::{Error, Result};
pub use transport::tcp::TcpTransport;
pub use transport::tls::TlsTransport;
pub use transport::udp::UdpTransport;
pub use transport::ws::WebSocketTransport;
pub use transport::{Transport, TransportEvent};

// Simplified helper functions
/// Bind a UDP transport to the specified address
pub async fn bind_udp(
    addr: std::net::SocketAddr,
) -> Result<(UdpTransport, tokio::sync::mpsc::Receiver<TransportEvent>)> {
    UdpTransport::bind(addr, None).await
}

/// Bind a TCP transport to the specified address
pub async fn bind_tcp(
    addr: std::net::SocketAddr,
) -> Result<(TcpTransport, tokio::sync::mpsc::Receiver<TransportEvent>)> {
    TcpTransport::bind(addr, None, None).await
}

/// Re-export of common types for easier use
pub mod prelude {
    pub use crate::{
        bind_tcp, bind_udp, events::TransportEventAdapter, factory::TransportFactory,
        manager::TransportManager, Error, Result, TcpTransport, TlsTransport, Transport,
        TransportEvent, UdpTransport, WebSocketTransport,
    };
}
