use std::io;
use thiserror::Error;

/// Result type for ICE operations
pub type Result<T> = std::result::Result<T, Error>;

/// Error types for ICE operations
#[derive(Debug, Error)]
pub enum Error {
    /// I/O error
    #[error("I/O error: {0}")]
    IoError(#[from] io::Error),

    /// STUN protocol error
    #[error("STUN error: {0}")]
    StunError(String),

    /// TURN protocol error
    #[error("TURN error: {0}")]
    TurnError(String),

    /// ICE protocol error
    #[error("ICE error: {0}")]
    IceError(String),

    /// Connection error
    #[error("Connection error: {0}")]
    ConnectionError(String),

    /// Configuration error
    #[error("Configuration error: {0}")]
    ConfigError(String),

    /// Invalid candidate
    #[error("Invalid candidate: {0}")]
    InvalidCandidate(String),

    /// Invalid state
    #[error("Invalid state: {0}")]
    InvalidState(String),

    /// Timeout
    #[error("Timeout: {0}")]
    Timeout(String),

    /// Authentication error
    #[error("Authentication error: {0}")]
    AuthError(String),

    /// Protocol error
    #[error("Protocol error: {0}")]
    ProtocolError(String),

    /// Other errors
    #[error("{0}")]
    Other(String),
} 