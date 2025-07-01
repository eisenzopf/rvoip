use std::io;
use std::net::SocketAddr;
use thiserror::Error;

/// Result type for SIP transport operations
pub type Result<T> = std::result::Result<T, Error>;

/// Error types for SIP transport operations
#[derive(Error, Debug)]
pub enum Error {
    /// Failed to bind to the specified address
    #[error("Failed to bind to {0}: {1}")]
    BindFailed(SocketAddr, io::Error),

    /// Failed to connect to the specified address
    #[error("Failed to connect to {0}: {1}")]
    ConnectFailed(SocketAddr, io::Error),

    /// Failed to send message to the specified address
    #[error("Failed to send message to {0}: {1}")]
    SendFailed(SocketAddr, io::Error),

    /// Failed to receive message
    #[error("Failed to receive message: {0}")]
    ReceiveFailed(io::Error),

    /// Failed to get local address
    #[error("Failed to get local address: {0}")]
    LocalAddrFailed(io::Error),

    /// Transport is closed
    #[error("Transport closed")]
    TransportClosed,

    /// Connection closed by peer
    #[error("Connection closed by peer: {0}")]
    ConnectionClosedByPeer(SocketAddr),

    /// Connection timed out
    #[error("Connection timed out: {0}")]
    ConnectionTimeout(SocketAddr),

    /// TLS handshake failed
    #[error("TLS handshake failed: {0}")]
    TlsHandshakeFailed(String),

    /// TLS certificate error
    #[error("TLS certificate error: {0}")]
    TlsCertificateError(String),

    /// WebSocket protocol error
    #[error("WebSocket protocol error: {0}")]
    WebSocketProtocolError(String),

    /// WebSocket handshake failed
    #[error("WebSocket handshake failed: {0}")]
    WebSocketHandshakeFailed(String),

    /// Message too large for transport
    #[error("Message too large for transport ({0} bytes)")]
    MessageTooLarge(usize),

    /// Partial send
    #[error("Partial send: {0} of {1} bytes sent")]
    PartialSend(usize, usize),

    /// I/O error
    #[error("I/O error: {0}")]
    IoError(#[from] io::Error),

    /// Invalid state
    #[error("Invalid state: {0}")]
    InvalidState(String),

    /// Connection pool exhausted
    #[error("Connection pool exhausted")]
    ConnectionPoolExhausted,

    /// Invalid address
    #[error("Invalid address: {0}")]
    InvalidAddress(String),

    /// DNS resolution failed
    #[error("DNS resolution failed: {0}")]
    DnsResolutionFailed(String),

    /// Protocol error
    #[error("Protocol error: {0}")]
    ProtocolError(String),

    /// Connection reset
    #[error("Connection reset")]
    ConnectionReset,

    /// Stream closed
    #[error("Stream closed")]
    StreamClosed,

    /// Invalid URI
    #[error("Invalid URI: {0}")]
    InvalidUri(String),

    /// Unsupported transport
    #[error("Unsupported transport: {0}")]
    UnsupportedTransport(String),

    /// Connection limit reached
    #[error("Connection limit reached")]
    ConnectionLimitReached,

    /// HTTP error
    #[error("HTTP error: {0}")]
    HttpError(String),

    /// Timeout
    #[error("Timeout")]
    Timeout,

    /// Failed to parse message
    #[error("Failed to parse message: {0}")]
    ParseError(String),

    /// Transport already bound
    #[error("Transport already bound")]
    AlreadyBound,

    /// Buffer capacity exceeded
    #[error("Buffer capacity exceeded")]
    BufferCapacityExceeded,

    /// Operation would block
    #[error("Operation would block")]
    WouldBlock,

    /// Not implemented
    #[error("Not implemented: {0}")]
    NotImplemented(String),
    
    /// Channel closed
    #[error("Channel closed")]
    ChannelClosed,
    
    /// Bind error
    #[error("Bind error: {0}")]
    BindError(String),
    
    /// Other error
    #[error("Other error: {0}")]
    Other(String),

    /// Internal error
    #[error("Internal error: {0}")]
    InternalError(String),
}

impl Error {
    /// Returns true if the error is related to a closed connection
    pub fn is_connection_closed(&self) -> bool {
        matches!(
            self,
            Error::TransportClosed
                | Error::ConnectionClosedByPeer(_)
                | Error::ConnectionReset
                | Error::StreamClosed
        )
    }

    /// Returns true if the error is related to a timeout
    pub fn is_timeout(&self) -> bool {
        matches!(self, Error::Timeout | Error::ConnectionTimeout(_))
    }

    /// Returns true if the error is related to DNS resolution
    pub fn is_dns_error(&self) -> bool {
        matches!(self, Error::DnsResolutionFailed(_))
    }

    /// Returns true if the error is related to TLS
    pub fn is_tls_error(&self) -> bool {
        matches!(
            self,
            Error::TlsHandshakeFailed(_) | Error::TlsCertificateError(_)
        )
    }

    /// Returns true if the error is related to WebSocket
    pub fn is_websocket_error(&self) -> bool {
        matches!(
            self,
            Error::WebSocketProtocolError(_) | Error::WebSocketHandshakeFailed(_)
        )
    }

    /// Returns true if the error is recoverable (retrying might succeed)
    pub fn is_recoverable(&self) -> bool {
        !matches!(
            self,
            Error::UnsupportedTransport(_)
                | Error::InvalidUri(_)
                | Error::MessageTooLarge(_)
                | Error::AlreadyBound
                | Error::InvalidState(_)
        )
    }
}

impl From<&str> for Error {
    fn from(s: &str) -> Self {
        Error::Other(s.to_string())
    }
}

impl From<String> for Error {
    fn from(s: String) -> Self {
        Error::Other(s)
    }
}

impl<T> From<tokio::sync::mpsc::error::SendError<T>> for Error {
    fn from(_: tokio::sync::mpsc::error::SendError<T>) -> Self {
        Error::ChannelClosed
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_error_categorization() {
        let closed_err = Error::TransportClosed;
        assert!(closed_err.is_connection_closed());
        assert!(!closed_err.is_timeout());
        
        let timeout_err = Error::Timeout;
        assert!(timeout_err.is_timeout());
        assert!(!timeout_err.is_dns_error());
        
        let dns_err = Error::DnsResolutionFailed("lookup failed".to_string());
        assert!(dns_err.is_dns_error());
        assert!(!dns_err.is_tls_error());
        
        let tls_err = Error::TlsHandshakeFailed("handshake failed".to_string());
        assert!(tls_err.is_tls_error());
        assert!(!tls_err.is_websocket_error());
        
        let ws_err = Error::WebSocketProtocolError("invalid frame".to_string());
        assert!(ws_err.is_websocket_error());
        assert!(!ws_err.is_connection_closed());
    }
    
    #[test]
    fn test_recoverable_errors() {
        // Recoverable errors
        assert!(Error::Timeout.is_recoverable());
        assert!(Error::ConnectionTimeout(SocketAddr::from(([127, 0, 0, 1], 5060))).is_recoverable());
        assert!(Error::IoError(io::Error::new(io::ErrorKind::ConnectionReset, "reset")).is_recoverable());
        
        // Non-recoverable errors
        assert!(!Error::UnsupportedTransport("xyz".to_string()).is_recoverable());
        assert!(!Error::InvalidUri("bad:uri".to_string()).is_recoverable());
        assert!(!Error::MessageTooLarge(100000).is_recoverable());
    }
} 