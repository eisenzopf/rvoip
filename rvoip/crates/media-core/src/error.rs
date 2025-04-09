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

    /// Other errors
    #[error("{0}")]
    Other(String),
}

impl From<&str> for Error {
    fn from(err: &str) -> Self {
        Error::Other(err.to_string())
    }
} 