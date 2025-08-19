//! Media error types

use std::fmt;

/// Media processing errors
#[derive(Debug, Clone, PartialEq)]
pub enum MediaError {
    /// Invalid input parameters
    InvalidInput(String),
    /// Configuration error
    ConfigError(String),
    /// Processing error
    ProcessingError(String),
    /// Format error
    FormatError(String),
    /// Buffer error
    BufferError(String),
    /// Quality error
    QualityError(String),
    /// Session error
    SessionError(String),
    /// General I/O error
    IoError(String),
}

impl fmt::Display for MediaError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            MediaError::InvalidInput(msg) => write!(f, "Invalid input: {}", msg),
            MediaError::ConfigError(msg) => write!(f, "Configuration error: {}", msg),
            MediaError::ProcessingError(msg) => write!(f, "Processing error: {}", msg),
            MediaError::FormatError(msg) => write!(f, "Format error: {}", msg),
            MediaError::BufferError(msg) => write!(f, "Buffer error: {}", msg),
            MediaError::QualityError(msg) => write!(f, "Quality error: {}", msg),
            MediaError::SessionError(msg) => write!(f, "Session error: {}", msg),
            MediaError::IoError(msg) => write!(f, "I/O error: {}", msg),
        }
    }
}

impl std::error::Error for MediaError {}

/// Result type for media operations
pub type MediaResult<T> = Result<T, MediaError>;