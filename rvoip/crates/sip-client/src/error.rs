use std::io;
use std::net::AddrParseError;
use thiserror::Error;

/// Result type for sip-client operations
pub type Result<T> = std::result::Result<T, Error>;

/// Error types for the SIP client
#[derive(Debug, thiserror::Error)]
pub enum Error {
    /// Transport error
    #[error("Transport error: {0}")]
    Transport(String),
    
    /// SIP protocol error
    #[error("SIP protocol error: {0}")]
    SipProtocol(String),
    
    /// SDP protocol error
    #[error("SDP protocol error: {0}")]
    SdpProtocol(String),
    
    /// Call error
    #[error("Call error: {0}")]
    Call(String),
    
    /// Media error
    #[error("Media error: {0}")]
    Media(String),
    
    /// Authentication error
    #[error("Authentication error: {0}")]
    Authentication(String),
    
    /// Registration error
    #[error("Registration error: {0}")]
    Registration(String),
    
    /// Configuration error
    #[error("Configuration error: {0}")]
    Configuration(String),
    
    /// Client error
    #[error("Client error: {0}")]
    Client(String),
    
    /// Unknown error
    #[error("Unknown error: {0}")]
    Unknown(String),

    /// SDP negotiation errors
    #[error("SDP negotiation error: {0}")]
    SdpNegotiation(String),

    /// Network errors
    #[error("Network error: {0}")]
    Network(#[from] io::Error),

    /// Address parsing errors
    #[error("Address parse error: {0}")]
    AddrParse(#[from] AddrParseError),

    /// Timeout errors
    #[error("Operation timed out: {0}")]
    Timeout(String),

    /// Invalid state transitions
    #[error("Invalid state transition: {0}")]
    InvalidState(String),

    /// Storage errors
    #[error("Storage error: {0}")]
    Storage(String),

    /// General errors
    #[error("{0}")]
    Other(String),

    /// SDP parsing error
    #[error("SDP parsing error: {0}")]
    SdpParsing(String),
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