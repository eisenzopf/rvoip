use thiserror::Error;
use std::io;

/// Error type for RTP operations
#[derive(Debug, Error, Clone)]
pub enum Error {
    /// Error when encoding RTP packet
    #[error("Failed to encode RTP packet: {0}")]
    EncodeError(String),

    /// Error when decoding RTP packet
    #[error("Failed to decode RTP packet: {0}")]
    DecodeError(String),

    /// Invalid packet format
    #[error("Invalid RTP packet format: {0}")]
    InvalidPacket(String),

    /// Buffer too small
    #[error("Buffer too small for RTP packet: need {required} but have {available}")]
    BufferTooSmall {
        required: usize,
        available: usize,
    },

    /// Invalid parameter for RTP operation
    #[error("Invalid parameter: {0}")]
    InvalidParameter(String),

    /// IO error when sending/receiving RTP packets
    #[error("IO error: {0}")]
    IoError(String),

    /// RTCP error
    #[error("RTCP error: {0}")]
    RtcpError(String),

    /// Session error
    #[error("RTP session error: {0}")]
    SessionError(String),
    
    /// Transport error
    #[error("Transport error: {0}")]
    Transport(String),
    
    /// Parsing error
    #[error("Parse error: {0}")]
    ParseError(String),
    
    /// SRTP error
    #[error("SRTP error: {0}")]
    SrtpError(String),
    
    /// Statistics error
    #[error("Statistics error: {0}")]
    StatsError(String),
    
    /// Timing error
    #[error("Timing error: {0}")]
    TimingError(String),

    /// Invalid protocol version
    #[error("Invalid protocol version: {0}")]
    InvalidProtocolVersion(String),

    /// Unsupported feature
    #[error("Unsupported feature: {0}")]
    UnsupportedFeature(String),
    
    /// Not implemented yet
    #[error("Not implemented: {0}")]
    NotImplemented(String),
    
    /// Invalid state for operation
    #[error("Invalid state: {0}")]
    InvalidState(String),
    
    /// DTLS handshake error
    #[error("DTLS handshake error: {0}")]
    DtlsHandshakeError(String),
    
    /// DTLS alert received
    #[error("DTLS alert received: {0}")]
    DtlsAlertReceived(String),
    
    /// Certificate validation error
    #[error("Certificate validation error: {0}")]
    CertificateValidationError(String),
    
    /// Cryptographic operation error
    #[error("Cryptographic error: {0}")]
    CryptoError(String),
    
    /// Invalid message format or content
    #[error("Invalid message: {0}")]
    InvalidMessage(String),
    
    /// Authentication failed
    #[error("Authentication failed: {0}")]
    AuthenticationFailed(String),
    
    /// Negotiation failed
    #[error("Negotiation failed: {0}")]
    NegotiationFailed(String),
    
    /// Packet too short
    #[error("Packet too short")]
    PacketTooShort,
    
    /// Timeout error
    #[error("Operation timed out: {0}")]
    Timeout(String),

    /// Serialization error
    #[error("Serialization error: {0}")]
    SerializationError(String),
}

impl From<io::Error> for Error {
    fn from(err: io::Error) -> Self {
        Error::IoError(err.to_string())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_error_display() {
        let encode_err = Error::EncodeError("test error".to_string());
        assert_eq!(encode_err.to_string(), "Failed to encode RTP packet: test error");
        
        let buffer_err = Error::BufferTooSmall { required: 100, available: 50 };
        assert_eq!(buffer_err.to_string(), "Buffer too small for RTP packet: need 100 but have 50");
        
        let io_err = Error::from(io::Error::new(io::ErrorKind::NotFound, "file not found"));
        assert!(io_err.to_string().contains("IO error"));
    }
} 