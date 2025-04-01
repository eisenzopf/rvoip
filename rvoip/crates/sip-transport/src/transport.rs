use std::fmt;
use std::net::SocketAddr;
use std::sync::Arc;

use rvoip_sip_core::Message;

use crate::error::Result;

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
} 