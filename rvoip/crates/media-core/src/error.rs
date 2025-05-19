use std::io;
use thiserror::Error;

/// Result type for media operations
pub type Result<T> = std::result::Result<T, Error>;

/// Error types for media operations
#[derive(Debug, Error)]
pub enum Error {
    /// I/O error
    #[error("I/O error: {0}")]
    IoError(#[from] io::Error),

    /// Media processing error
    #[error("Media processing error: {0}")]
    Media(String),

    /// Network error
    #[error("Network error: {0}")]
    Network(String),

    /// Codec error
    #[error("Codec error: {0}")]
    Codec(String),

    /// Format error
    #[error("Format error: {0}")]
    Format(String),

    /// Invalid format
    #[error("Invalid format: {0}")]
    InvalidFormat(String),

    /// SRTP error
    #[error("SRTP error: {0}")]
    Srtp(String),

    /// DTLS error
    #[error("DTLS error: {0}")]
    Dtls(String),

    /// ICE error
    #[error("ICE error: {0}")]
    Ice(String),

    /// Invalid state
    #[error("Invalid state: {0}")]
    InvalidState(String),

    /// Invalid parameter
    #[error("Invalid parameter: {0}")]
    InvalidParameter(String),

    /// Timeout
    #[error("Timeout: {0}")]
    Timeout(String),
    
    /// Session not found
    #[error("Session not found: {0}")]
    SessionNotFound(String),
    
    /// Dialog not found
    #[error("Dialog not found: {0}")]
    DialogNotFound(String),
    
    /// No codec selected
    #[error("No codec selected")]
    NoCodec,
    
    /// Unsupported codec
    #[error("Unsupported codec: {0}")]
    UnsupportedCodec(String),
    
    /// No remote address
    #[error("No remote address set")]
    NoRemoteAddress,
    
    /// Event channel full
    #[error("Event channel full")]
    EventChannelFull,
    
    /// RTP error
    #[error("RTP error: {0}")]
    RtpError(String),
    
    /// Not implemented
    #[error("Not implemented: {0}")]
    NotImplemented(String),

    /// Other errors
    #[error("{0}")]
    Other(String),
}

impl From<&str> for Error {
    fn from(err: &str) -> Self {
        Error::Other(err.to_string())
    }
} 