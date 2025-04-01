//! SIP transport layer implementation for the rvoip stack
//!
//! This crate provides the transport layer for SIP messages, including
//! UDP, TCP, TLS, and WebSocket transports.

mod error;
mod transport;
mod udp;

pub use error::{Error, Result};
pub use transport::{Transport, TransportEvent};
pub use udp::UdpTransport;

/// Re-export of common types and functions
pub mod prelude {
    pub use super::{Error, Result, Transport, TransportEvent, UdpTransport};
} 