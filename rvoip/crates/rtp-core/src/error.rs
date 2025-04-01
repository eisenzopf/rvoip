use thiserror::Error;

/// Error type for RTP operations
#[derive(Debug, Error)]
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
    IoError(#[from] std::io::Error),

    /// RTCP error
    #[error("RTCP error: {0}")]
    RtcpError(String),

    /// Session error
    #[error("RTP session error: {0}")]
    SessionError(String),
} 