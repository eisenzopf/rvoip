use std::io;
use std::net::AddrParseError;
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

    /// Security error
    #[error("Security error: {0}")]
    Security(String),

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

    /// Invalid data
    #[error("Invalid data: {0}")]
    InvalidData(String),

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
    
    /// No codec selected (alias for compatibility)
    #[error("No codec selected")]
    NoCodecSelected,
    
    /// Codec not found
    #[error("Codec not found: {0}")]
    CodecNotFound(String),
    
    /// Unsupported codec
    #[error("Unsupported codec: {0}")]
    UnsupportedCodec(String),
    
    /// Device not found
    #[error("Device not found: {0}")]
    DeviceNotFound(String),
    
    /// No remote address
    #[error("No remote address set")]
    NoRemoteAddress,
    
    /// Event channel full
    #[error("Event channel full")]
    EventChannelFull,
    
    /// Channel send error
    #[error("Channel send error: {0}")]
    ChannelSendError(String),
    
    /// Transport error
    #[error("Transport error: {0}")]
    TransportError(String),
    
    /// Not initialized
    #[error("Not initialized: {0}")]
    NotInitialized(String),
    
    /// RTP error
    #[error("RTP error: {0}")]
    RtpError(String),
    
    /// Not implemented
    #[error("Not implemented: {0}")]
    NotImplemented(String),

    /// Other errors
    #[error("{0}")]
    Other(String),
    
    /// Invalid argument
    #[error("Invalid argument: {0}")]
    InvalidArgument(String),
    
    /// Stream not found
    #[error("Stream not found: {0}")]
    StreamNotFound(String),
    
    /// Unsupported format
    #[error("Unsupported format: {0}")]
    UnsupportedFormat(String),
    
    /// Insufficient data
    #[error("Insufficient data: {0}")]
    InsufficientData(String),
    
    /// Format mismatch
    #[error("Format mismatch: {0}")]
    FormatMismatch(String),
    
    /// Encoding failed
    #[error("Encoding failed: {0}")]
    EncodingFailed(String),
}

impl From<&str> for Error {
    fn from(err: &str) -> Self {
        Error::Other(err.to_string())
    }
}

impl From<AddrParseError> for Error {
    fn from(err: AddrParseError) -> Self {
        Error::InvalidParameter(format!("Invalid address: {}", err))
    }
} 