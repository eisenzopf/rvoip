use std::io;
use std::net::AddrParseError;
use thiserror::Error;

/// Result type for sip-client operations
pub type Result<T> = std::result::Result<T, Error>;

/// Error types for sip-client operations
#[derive(Error, Debug)]
pub enum Error {
    /// SIP protocol errors
    #[error("SIP protocol error: {0}")]
    SipProtocol(String),

    /// SDP negotiation errors
    #[error("SDP negotiation error: {0}")]
    SdpNegotiation(String),

    /// Media errors
    #[error("Media error: {0}")]
    Media(String),

    /// Registration errors
    #[error("Registration error: {0}")]
    Registration(String),

    /// Call errors
    #[error("Call error: {0}")]
    Call(String),

    /// Network errors
    #[error("Network error: {0}")]
    Network(#[from] io::Error),

    /// Address parsing errors
    #[error("Address parse error: {0}")]
    AddrParse(#[from] AddrParseError),

    /// Transport errors
    #[error("Transport error: {0}")]
    Transport(String),

    /// Authentication errors
    #[error("Authentication error: {0}")]
    Authentication(String),

    /// Configuration errors
    #[error("Configuration error: {0}")]
    Configuration(String),

    /// Timeout errors
    #[error("Operation timed out: {0}")]
    Timeout(String),

    /// Invalid state transitions
    #[error("Invalid state transition: {0}")]
    InvalidState(String),

    /// General errors
    #[error("{0}")]
    Other(String),
}

impl From<anyhow::Error> for Error {
    fn from(err: anyhow::Error) -> Self {
        Error::Other(err.to_string())
    }
}

impl From<&str> for Error {
    fn from(s: &str) -> Self {
        Error::Other(s.to_string())
    }
} 