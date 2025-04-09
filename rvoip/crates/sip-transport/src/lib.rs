//! SIP transport layer implementation for the rvoip stack
//!
//! This crate provides the transport layer for SIP messages, including
//! UDP, TCP, TLS, and WebSocket transports.

pub mod transport;
pub mod error;
pub mod udp;
pub mod tls;

pub use transport::{Transport, TransportEvent};
pub use error::{Error, Result};
pub use udp::UdpTransport;
pub use tls::TlsTransport;

/// Simplified bind function for UdpTransport
pub async fn bind_udp(addr: std::net::SocketAddr) -> Result<(UdpTransport, tokio::sync::mpsc::Receiver<TransportEvent>)> {
    UdpTransport::bind(addr, None).await
}

/// Re-export of common types for easier use
pub mod prelude {
    pub use super::{Error, Result, Transport, TransportEvent, UdpTransport, bind_udp};
}

#[cfg(test)]
mod tests {
    #[test]
    fn it_works() {
        assert_eq!(2 + 2, 4);
    }
} 