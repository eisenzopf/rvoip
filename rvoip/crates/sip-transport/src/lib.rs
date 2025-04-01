//! SIP transport layer implementation for the rvoip stack
//!
//! This crate provides the transport layer for SIP messages, including
//! UDP, TCP, TLS, and WebSocket transports.

mod error;
pub mod transport;
mod udp;

pub use error::{Error, Result};
pub use transport::{Transport, TransportEvent};
pub use udp::UdpTransport;

/// Simplified bind function for UdpTransport
pub async fn bind_udp(addr: std::net::SocketAddr) -> Result<(UdpTransport, tokio::sync::mpsc::Receiver<TransportEvent>)> {
    UdpTransport::bind(addr, None).await
}

/// Re-export of common types for easier use
pub mod prelude {
    pub use super::{Error, Result, Transport, TransportEvent, UdpTransport, bind_udp};
} 