//! Error types for media-core operations
//!
//! This module defines comprehensive error handling for all media processing
//! operations, codec management, and integration with other crates.

use std::fmt;
use thiserror::Error;

/// Result type alias for media-core operations
pub type Result<T> = std::result::Result<T, Error>;

/// Comprehensive error types for media-core
#[derive(Debug, Error)]
pub enum Error {
    /// Media session errors
    #[error("Media session error: {0}")]
    MediaSession(#[from] MediaSessionError),
    
    /// Codec-related errors
    #[error("Codec error: {0}")]
    Codec(#[from] CodecError),
    
    /// Audio processing errors
    #[error("Audio processing error: {0}")]
    AudioProcessing(#[from] AudioProcessingError),
    
    /// Quality monitoring errors
    #[error("Quality monitoring error: {0}")]
    Quality(#[from] QualityError),
    
    /// Buffer management errors
    #[error("Buffer error: {0}")]
    Buffer(#[from] BufferError),
    
    /// Integration errors with other crates
    #[error("Integration error: {0}")]
    Integration(#[from] IntegrationError),
    
    /// Configuration errors
    #[error("Configuration error: {0}")]
    Config(String),
    
    /// I/O errors
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),
}

/// Media session specific errors
#[derive(Debug, Error)]
pub enum MediaSessionError {
    #[error("Session not found: {session_id}")]
    NotFound { session_id: String },
    
    #[error("Session already exists: {session_id}")]
    AlreadyExists { session_id: String },
    
    #[error("Invalid session state: {state}")]
    InvalidState { state: String },
    
    #[error("Session creation failed: {reason}")]
    CreationFailed { reason: String },
    
    #[error("Session operation failed: {operation}")]
    OperationFailed { operation: String },
}

/// Codec-related errors
#[derive(Debug, Error)]
pub enum CodecError {
    #[error("Unsupported payload type: {payload_type}")]
    UnsupportedPayloadType { payload_type: u8 },
    
    #[error("Codec not found: {name}")]
    NotFound { name: String },
    
    #[error("Encoding failed: {reason}")]
    EncodingFailed { reason: String },
    
    #[error("Decoding failed: {reason}")]
    DecodingFailed { reason: String },
    
    #[error("Invalid codec parameters: {details}")]
    InvalidParameters { details: String },
    
    #[error("Codec negotiation failed: {reason}")]
    NegotiationFailed { reason: String },
    
    #[error("Transcoding not supported: {from_codec} to {to_codec}")]
    TranscodingUnsupported { from_codec: String, to_codec: String },
}

/// Audio processing errors
#[derive(Debug, Error)]
pub enum AudioProcessingError {
    #[error("Invalid audio format: {details}")]
    InvalidFormat { details: String },
    
    #[error("Sample rate conversion failed: {from_rate} to {to_rate}")]
    ResamplingFailed { from_rate: u32, to_rate: u32 },
    
    #[error("Channel conversion failed: {from_channels} to {to_channels}")]
    ChannelConversionFailed { from_channels: u8, to_channels: u8 },
    
    #[error("Processing component not available: {component}")]
    ComponentNotAvailable { component: String },
    
    #[error("Processing failed: {reason}")]
    ProcessingFailed { reason: String },
}

/// Quality monitoring errors
#[derive(Debug, Error)]
pub enum QualityError {
    #[error("Metrics collection failed: {reason}")]
    MetricsCollectionFailed { reason: String },
    
    #[error("Quality adaptation failed: {reason}")]
    AdaptationFailed { reason: String },
    
    #[error("Threshold violation: {metric} = {value}, threshold = {threshold}")]
    ThresholdViolation { metric: String, value: f64, threshold: f64 },
}

/// Buffer management errors
#[derive(Debug, Error)]
pub enum BufferError {
    #[error("Buffer overflow: {buffer_type}")]
    Overflow { buffer_type: String },
    
    #[error("Buffer underflow: {buffer_type}")]
    Underflow { buffer_type: String },
    
    #[error("Invalid buffer size: {size}")]
    InvalidSize { size: usize },
    
    #[error("Buffer allocation failed: {reason}")]
    AllocationFailed { reason: String },
}

/// Integration errors with other crates
#[derive(Debug, Error)]
pub enum IntegrationError {
    #[error("RTP-core integration error: {details}")]
    RtpCore { details: String },
    
    #[error("Session-core integration error: {details}")]
    SessionCore { details: String },
    
    #[error("Bridge communication failed: {bridge}")]
    BridgeFailed { bridge: String },
    
    #[error("Event handling failed: {event_type}")]
    EventHandlingFailed { event_type: String },
}

// Convenience constructors for common error patterns
impl Error {
    /// Create a configuration error
    pub fn config(msg: impl Into<String>) -> Self {
        Self::Config(msg.into())
    }
    
    /// Create a media session not found error
    pub fn session_not_found(session_id: impl Into<String>) -> Self {
        Self::MediaSession(MediaSessionError::NotFound {
            session_id: session_id.into(),
        })
    }
    
    /// Create a codec not found error
    pub fn codec_not_found(name: impl Into<String>) -> Self {
        Self::Codec(CodecError::NotFound {
            name: name.into(),
        })
    }
    
    /// Create an unsupported payload type error
    pub fn unsupported_payload_type(payload_type: u8) -> Self {
        Self::Codec(CodecError::UnsupportedPayloadType { payload_type })
    }
} 