//! Error handling for the codec library
//!
//! This module defines comprehensive error types that can occur during
//! codec operations, providing detailed information for debugging and
//! error recovery.

#![allow(missing_docs)]

use std::fmt;
use thiserror::Error;

/// Result type alias for codec operations
pub type Result<T> = std::result::Result<T, CodecError>;

/// Comprehensive error type for codec operations
#[derive(Error, Debug)]
pub enum CodecError {
    /// Invalid codec configuration
    #[error("Invalid codec configuration: {details}")]
    InvalidConfig { details: String },

    /// Unsupported codec type
    #[error("Unsupported codec type: {codec_type}")]
    UnsupportedCodec { codec_type: String },

    /// Invalid audio format
    #[error("Invalid audio format: {details}")]
    InvalidFormat { details: String },

    /// Invalid frame size
    #[error("Invalid frame size: expected {expected}, got {actual}")]
    InvalidFrameSize { expected: usize, actual: usize },

    /// Invalid sample rate
    #[error("Invalid sample rate: {rate}Hz (supported: {supported:?})")]
    InvalidSampleRate { rate: u32, supported: Vec<u32> },

    /// Invalid channel count
    #[error("Invalid channel count: {channels} (supported: {supported:?})")]
    InvalidChannelCount { channels: u8, supported: Vec<u8> },

    /// Invalid bitrate
    #[error("Invalid bitrate: {bitrate}bps (range: {min}-{max})")]
    InvalidBitrate { bitrate: u32, min: u32, max: u32 },

    /// Encoding operation failed
    #[error("Encoding failed: {reason}")]
    EncodingFailed { reason: String },

    /// Decoding operation failed
    #[error("Decoding failed: {reason}")]
    DecodingFailed { reason: String },

    /// Buffer too small for operation
    #[error("Buffer too small: need {needed} bytes, got {actual}")]
    BufferTooSmall { needed: usize, actual: usize },

    /// Buffer overflow during operation
    #[error("Buffer overflow: attempted to write {size} bytes to {capacity} byte buffer")]
    BufferOverflow { size: usize, capacity: usize },

    /// Codec initialization failed
    #[error("Codec initialization failed: {reason}")]
    InitializationFailed { reason: String },

    /// Codec reset failed
    #[error("Codec reset failed: {reason}")]
    ResetFailed { reason: String },

    /// Invalid payload data
    #[error("Invalid payload data: {details}")]
    InvalidPayload { details: String },

    /// Codec not found
    #[error("Codec not found: {name}")]
    CodecNotFound { name: String },

    /// Feature not enabled
    #[error("Feature not enabled: {feature} (enable with --features {feature})")]
    FeatureNotEnabled { feature: String },

    /// SIMD operation failed
    #[error("SIMD operation failed: {reason}")]
    SimdFailed { reason: String },

    /// Math operation failed (overflow, underflow, etc.)
    #[error("Math operation failed: {operation} - {reason}")]
    MathError { operation: String, reason: String },

    /// I/O operation failed
    #[error("I/O operation failed: {reason}")]
    IoError { reason: String },

    /// External library error
    #[error("External library error: {library} - {error}")]
    ExternalLibraryError { library: String, error: String },

    /// Internal error (should not occur in normal operation)
    #[error("Internal error: {message} (this is a bug, please report it)")]
    InternalError { message: String },
}

impl CodecError {
    /// Create a new invalid configuration error
    pub fn invalid_config(details: impl Into<String>) -> Self {
        Self::InvalidConfig {
            details: details.into(),
        }
    }

    /// Create a new unsupported codec error
    pub fn unsupported_codec(codec_type: impl Into<String>) -> Self {
        Self::UnsupportedCodec {
            codec_type: codec_type.into(),
        }
    }

    /// Create a new invalid format error
    pub fn invalid_format(details: impl Into<String>) -> Self {
        Self::InvalidFormat {
            details: details.into(),
        }
    }

    /// Create a new encoding failed error
    pub fn encoding_failed(reason: impl Into<String>) -> Self {
        Self::EncodingFailed {
            reason: reason.into(),
        }
    }

    /// Create a new decoding failed error
    pub fn decoding_failed(reason: impl Into<String>) -> Self {
        Self::DecodingFailed {
            reason: reason.into(),
        }
    }

    /// Create a new initialization failed error
    pub fn initialization_failed(reason: impl Into<String>) -> Self {
        Self::InitializationFailed {
            reason: reason.into(),
        }
    }

    /// Create a new feature not enabled error
    pub fn feature_not_enabled(feature: impl Into<String>) -> Self {
        Self::FeatureNotEnabled {
            feature: feature.into(),
        }
    }

    /// Create a new internal error
    pub fn internal_error(message: impl Into<String>) -> Self {
        Self::InternalError {
            message: message.into(),
        }
    }

    /// Check if this error is recoverable
    pub fn is_recoverable(&self) -> bool {
        match self {
            // Configuration errors are not recoverable
            Self::InvalidConfig { .. } 
            | Self::UnsupportedCodec { .. }
            | Self::InvalidFormat { .. }
            | Self::InvalidSampleRate { .. }
            | Self::InvalidChannelCount { .. }
            | Self::InvalidBitrate { .. }
            | Self::FeatureNotEnabled { .. }
            | Self::CodecNotFound { .. }
            | Self::InternalError { .. } => false,

            // Operational errors may be recoverable
            Self::InvalidFrameSize { .. }
            | Self::EncodingFailed { .. }
            | Self::DecodingFailed { .. }
            | Self::BufferTooSmall { .. }
            | Self::BufferOverflow { .. }
            | Self::InvalidPayload { .. }
            | Self::SimdFailed { .. }
            | Self::MathError { .. }
            | Self::IoError { .. }
            | Self::ExternalLibraryError { .. } => true,

            // Reset and initialization errors depend on the specific cause
            Self::InitializationFailed { .. }
            | Self::ResetFailed { .. } => false,
        }
    }

    /// Get the error category
    pub fn category(&self) -> ErrorCategory {
        match self {
            Self::InvalidConfig { .. }
            | Self::UnsupportedCodec { .. }
            | Self::InvalidFormat { .. }
            | Self::InvalidSampleRate { .. }
            | Self::InvalidChannelCount { .. }
            | Self::InvalidBitrate { .. }
            | Self::FeatureNotEnabled { .. }
            | Self::CodecNotFound { .. } => ErrorCategory::Configuration,

            Self::EncodingFailed { .. }
            | Self::DecodingFailed { .. }
            | Self::InvalidFrameSize { .. }
            | Self::InvalidPayload { .. } => ErrorCategory::Processing,

            Self::BufferTooSmall { .. }
            | Self::BufferOverflow { .. } => ErrorCategory::Memory,

            Self::InitializationFailed { .. }
            | Self::ResetFailed { .. } => ErrorCategory::Initialization,

            Self::SimdFailed { .. }
            | Self::MathError { .. } => ErrorCategory::Computation,

            Self::IoError { .. } => ErrorCategory::Io,

            Self::ExternalLibraryError { .. } => ErrorCategory::External,

            Self::InternalError { .. } => ErrorCategory::Internal,
        }
    }
}

/// Error category for grouping related errors
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCategory {
    /// Configuration and parameter errors
    Configuration,
    /// Audio processing errors
    Processing,
    /// Memory management errors
    Memory,
    /// Initialization and setup errors
    Initialization,
    /// Computational errors (SIMD, math, etc.)
    Computation,
    /// I/O related errors
    Io,
    /// External library errors
    External,
    /// Internal library errors
    Internal,
}

impl fmt::Display for ErrorCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Configuration => write!(f, "Configuration"),
            Self::Processing => write!(f, "Processing"),
            Self::Memory => write!(f, "Memory"),
            Self::Initialization => write!(f, "Initialization"),
            Self::Computation => write!(f, "Computation"),
            Self::Io => write!(f, "I/O"),
            Self::External => write!(f, "External"),
            Self::Internal => write!(f, "Internal"),
        }
    }
}

/// Convert from I/O errors
impl From<std::io::Error> for CodecError {
    fn from(error: std::io::Error) -> Self {
        Self::IoError {
            reason: error.to_string(),
        }
    }
}

/// Convert from parsing errors
impl From<std::num::ParseIntError> for CodecError {
    fn from(error: std::num::ParseIntError) -> Self {
        Self::MathError {
            operation: "parse_int".to_string(),
            reason: error.to_string(),
        }
    }
}

/// Convert from parsing errors
impl From<std::num::ParseFloatError> for CodecError {
    fn from(error: std::num::ParseFloatError) -> Self {
        Self::MathError {
            operation: "parse_float".to_string(),
            reason: error.to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_creation() {
        let err = CodecError::invalid_config("test message");
        assert!(matches!(err, CodecError::InvalidConfig { .. }));
        assert_eq!(err.category(), ErrorCategory::Configuration);
    }

    #[test]
    fn test_error_recoverability() {
        let recoverable = CodecError::EncodingFailed {
            reason: "test".to_string(),
        };
        assert!(recoverable.is_recoverable());

        let non_recoverable = CodecError::InvalidConfig {
            details: "test".to_string(),
        };
        assert!(!non_recoverable.is_recoverable());
    }

    #[test]
    fn test_error_categories() {
        assert_eq!(
            CodecError::invalid_config("test").category(),
            ErrorCategory::Configuration
        );
        assert_eq!(
            CodecError::encoding_failed("test").category(),
            ErrorCategory::Processing
        );
        assert_eq!(
            CodecError::BufferTooSmall { needed: 100, actual: 50 }.category(),
            ErrorCategory::Memory
        );
    }

    #[test]
    fn test_error_display() {
        let err = CodecError::InvalidFrameSize {
            expected: 160,
            actual: 80,
        };
        let display = format!("{}", err);
        assert!(display.contains("expected 160"));
        assert!(display.contains("got 80"));
    }

    #[test]
    fn test_error_conversion() {
        let io_err = std::io::Error::new(std::io::ErrorKind::NotFound, "file not found");
        let codec_err: CodecError = io_err.into();
        assert!(matches!(codec_err, CodecError::IoError { .. }));
    }
} 