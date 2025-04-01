use thiserror::Error;

/// Error type for media operations
#[derive(Debug, Error)]
pub enum Error {
    /// Error when encoding audio
    #[error("Failed to encode audio: {0}")]
    EncodeError(String),

    /// Error when decoding audio
    #[error("Failed to decode audio: {0}")]
    DecodeError(String),

    /// Invalid audio format
    #[error("Invalid audio format: {0}")]
    InvalidFormat(String),

    /// Buffer too small
    #[error("Buffer too small: need {required} but have {available}")]
    BufferTooSmall {
        required: usize,
        available: usize,
    },

    /// Unsupported codec feature or operation
    #[error("Unsupported codec feature: {0}")]
    UnsupportedFeature(String),

    /// I/O error
    #[error("I/O error: {0}")]
    IoError(#[from] std::io::Error),
} 