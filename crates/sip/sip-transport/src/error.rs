use std::fmt;
use std::io;
use std::net::SocketAddr;

/// Result type for SIP transport operations
pub type Result<T> = std::result::Result<T, Error>;

/// Error types for SIP transport operations
pub enum Error {
    /// Failed to bind to the specified address
    BindFailed(SocketAddr, io::Error),

    /// Failed to connect to the specified address
    ConnectFailed(SocketAddr, io::Error),

    /// Failed to send message to the specified address
    SendFailed(SocketAddr, io::Error),

    /// Failed to receive message
    ReceiveFailed(io::Error),

    /// Failed to get local address
    LocalAddrFailed(io::Error),

    /// Transport is closed
    TransportClosed,

    /// Connection closed by peer
    ConnectionClosedByPeer(SocketAddr),

    /// Connection timed out
    ConnectionTimeout(SocketAddr),

    /// TLS handshake failed
    TlsHandshakeFailed(String),

    /// TLS certificate error
    TlsCertificateError(String),

    /// WebSocket protocol error
    WebSocketProtocolError(String),

    /// WebSocket handshake failed
    WebSocketHandshakeFailed(String),

    /// Message too large for transport
    MessageTooLarge(usize),

    /// Partial send
    PartialSend(usize, usize),

    /// I/O error
    IoError(io::Error),

    /// Invalid state
    InvalidState(String),

    /// Connection pool exhausted
    ConnectionPoolExhausted,

    /// Invalid address
    InvalidAddress(String),

    /// DNS resolution failed
    DnsResolutionFailed(String),

    /// Protocol error
    ProtocolError(String),

    /// Connection reset
    ConnectionReset,

    /// Stream closed
    StreamClosed,

    /// Invalid URI
    InvalidUri(String),

    /// Unsupported transport
    UnsupportedTransport(String),

    /// Connection limit reached
    ConnectionLimitReached,

    /// HTTP error
    HttpError(String),

    /// Timeout
    Timeout,

    /// Failed to parse message
    ParseError(String),

    /// Transport already bound
    AlreadyBound,

    /// Buffer capacity exceeded
    BufferCapacityExceeded,

    /// Operation would block
    WouldBlock,

    /// Not implemented
    NotImplemented(String),

    /// Channel closed
    ChannelClosed,

    /// Bind error
    BindError(String),

    /// Other error
    Other(String),

    /// Internal error
    InternalError(String),
}

impl fmt::Display for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::BindFailed(address, error) => write!(
                formatter,
                "failed to bind to {address} (I/O class {:?})",
                error.kind()
            ),
            Self::ConnectFailed(address, error) => write!(
                formatter,
                "failed to connect to {address} (I/O class {:?})",
                error.kind()
            ),
            Self::SendFailed(address, error) => write!(
                formatter,
                "failed to send to {address} (I/O class {:?})",
                error.kind()
            ),
            Self::ReceiveFailed(error) => {
                write!(formatter, "receive failed (I/O class {:?})", error.kind())
            }
            Self::LocalAddrFailed(error) => write!(
                formatter,
                "local-address lookup failed (I/O class {:?})",
                error.kind()
            ),
            Self::TransportClosed => formatter.write_str("transport closed"),
            Self::ConnectionClosedByPeer(address) => {
                write!(formatter, "connection closed by peer {address}")
            }
            Self::ConnectionTimeout(address) => {
                write!(formatter, "connection timed out for {address}")
            }
            Self::TlsHandshakeFailed(_) => formatter.write_str("TLS handshake failed"),
            Self::TlsCertificateError(_) => formatter.write_str("TLS certificate error"),
            Self::WebSocketProtocolError(_) => formatter.write_str("WebSocket protocol error"),
            Self::WebSocketHandshakeFailed(_) => formatter.write_str("WebSocket handshake failed"),
            Self::MessageTooLarge(bytes) => {
                write!(formatter, "message too large for transport ({bytes} bytes)")
            }
            Self::PartialSend(sent, total) => {
                write!(formatter, "partial send: {sent} of {total} bytes")
            }
            Self::IoError(error) => {
                write!(formatter, "I/O error class {:?}", error.kind())
            }
            Self::InvalidState(_) => formatter.write_str("invalid transport state"),
            Self::ConnectionPoolExhausted => formatter.write_str("connection pool exhausted"),
            Self::InvalidAddress(_) => formatter.write_str("invalid address"),
            Self::DnsResolutionFailed(_) => formatter.write_str("DNS resolution failed"),
            Self::ProtocolError(_) => formatter.write_str("transport protocol error"),
            Self::ConnectionReset => formatter.write_str("connection reset"),
            Self::StreamClosed => formatter.write_str("stream closed"),
            Self::InvalidUri(_) => formatter.write_str("invalid URI"),
            Self::UnsupportedTransport(_) => formatter.write_str("unsupported transport"),
            Self::ConnectionLimitReached => formatter.write_str("connection limit reached"),
            Self::HttpError(_) => formatter.write_str("HTTP transport error"),
            Self::Timeout => formatter.write_str("timeout"),
            Self::ParseError(_) => formatter.write_str("failed to parse SIP message"),
            Self::AlreadyBound => formatter.write_str("transport already bound"),
            Self::BufferCapacityExceeded => formatter.write_str("buffer capacity exceeded"),
            Self::WouldBlock => formatter.write_str("operation would block"),
            Self::NotImplemented(_) => formatter.write_str("transport operation not implemented"),
            Self::ChannelClosed => formatter.write_str("transport channel closed"),
            Self::BindError(_) => formatter.write_str("bind error"),
            Self::Other(_) => formatter.write_str("transport operation failed"),
            Self::InternalError(_) => formatter.write_str("internal transport error"),
        }
    }
}

impl fmt::Debug for Error {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, formatter)
    }
}

impl std::error::Error for Error {}

impl From<io::Error> for Error {
    fn from(error: io::Error) -> Self {
        Self::IoError(error)
    }
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
        assert!(
            Error::ConnectionTimeout(SocketAddr::from(([127, 0, 0, 1], 5060))).is_recoverable()
        );
        assert!(
            Error::IoError(io::Error::new(io::ErrorKind::ConnectionReset, "reset"))
                .is_recoverable()
        );

        // Non-recoverable errors
        assert!(!Error::UnsupportedTransport("xyz".to_string()).is_recoverable());
        assert!(!Error::InvalidUri("bad:uri".to_string()).is_recoverable());
        assert!(!Error::MessageTooLarge(100000).is_recoverable());
    }

    #[test]
    fn error_diagnostics_never_render_retained_lower_values() {
        const SECRET: &str = "transport-error-secret-canary.example";
        let errors = [
            Error::TlsHandshakeFailed(SECRET.to_string()),
            Error::TlsCertificateError(SECRET.to_string()),
            Error::WebSocketProtocolError(SECRET.to_string()),
            Error::WebSocketHandshakeFailed(SECRET.to_string()),
            Error::InvalidState(SECRET.to_string()),
            Error::InvalidAddress(SECRET.to_string()),
            Error::DnsResolutionFailed(SECRET.to_string()),
            Error::ProtocolError(SECRET.to_string()),
            Error::InvalidUri(SECRET.to_string()),
            Error::UnsupportedTransport(SECRET.to_string()),
            Error::HttpError(SECRET.to_string()),
            Error::ParseError(SECRET.to_string()),
            Error::NotImplemented(SECRET.to_string()),
            Error::BindError(SECRET.to_string()),
            Error::Other(SECRET.to_string()),
            Error::InternalError(SECRET.to_string()),
            Error::IoError(io::Error::other(SECRET)),
        ];

        for error in errors {
            for rendered in [error.to_string(), format!("{error:?}")] {
                assert!(!rendered.contains(SECRET), "error leaked: {rendered}");
            }
        }
    }
}
