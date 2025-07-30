//! Error types for the audio-core library
//!
//! This module defines comprehensive error handling for all audio operations,
//! including device management, format conversion, codec processing, and RTP integration.

use std::fmt;

/// Result type for audio operations
pub type AudioResult<T> = std::result::Result<T, AudioError>;

/// Comprehensive error type for audio operations
#[derive(Debug, thiserror::Error)]
pub enum AudioError {
    /// Audio device not found or unavailable
    #[error("Audio device not found: {device_id}")]
    DeviceNotFound {
        /// The device identifier that was not found
        device_id: String,
    },

    /// Audio device is already in use
    #[error("Audio device already in use: {device_id}")]
    DeviceInUse {
        /// The device identifier that is in use
        device_id: String,
    },

    /// Audio device disconnected unexpectedly
    #[error("Audio device disconnected: {device_id}")]
    DeviceDisconnected {
        /// The device identifier that was disconnected
        device_id: String,
    },

    /// Audio format not supported by device or codec
    #[error("Audio format not supported: {format} by {component}")]
    FormatNotSupported {
        /// The unsupported format description
        format: String,
        /// The component that doesn't support the format
        component: String,
    },

    /// Audio format conversion failed
    #[error("Format conversion failed: {source_format} -> {target_format}: {reason}")]
    FormatConversionFailed {
        /// Source format description
        source_format: String,
        /// Target format description
        target_format: String,
        /// Reason for conversion failure
        reason: String,
    },


    /// Audio buffer overflow/underflow
    #[error("Audio buffer {buffer_type} {error_type}: {details}")]
    BufferError {
        /// Type of buffer (capture, playback, jitter, etc.)
        buffer_type: String,
        /// Type of error (overflow, underflow, corruption)
        error_type: String,
        /// Additional error details
        details: String,
    },

    /// Audio pipeline error
    #[error("Audio pipeline error in {stage}: {reason}")]
    PipelineError {
        /// Pipeline stage where error occurred
        stage: String,
        /// Reason for the error
        reason: String,
    },

    /// RTP payload processing error
    #[error("RTP payload error: {operation} failed: {reason}")]
    RtpPayloadError {
        /// Operation that failed (encode/decode/parse)
        operation: String,
        /// Reason for the failure
        reason: String,
    },

    /// Audio synchronization error
    #[error("Audio synchronization error: {reason}")]
    SynchronizationError {
        /// Reason for synchronization failure
        reason: String,
    },

    /// Platform-specific audio error
    #[error("Platform audio error: {message}")]
    PlatformError {
        /// Platform-specific error message
        message: String,
    },

    /// Configuration error
    #[error("Configuration error in {component}: {reason}")]
    ConfigurationError {
        /// Component with configuration issue
        component: String,
        /// Reason for configuration error
        reason: String,
    },

    /// I/O error during audio operations
    #[error("Audio I/O error: {operation}: {message}")]
    IoError {
        /// The I/O operation that failed
        operation: String,
        /// The I/O error message
        message: String,
    },

    /// Session integration error
    #[error("Session integration error: {reason}")]
    SessionError {
        /// Reason for session integration failure
        reason: String,
    },

    /// Timeout during audio operation
    #[error("Audio operation timed out: {operation} after {timeout_ms}ms")]
    Timeout {
        /// Operation that timed out
        operation: String,
        /// Timeout duration in milliseconds
        timeout_ms: u64,
    },

    /// Resource exhaustion (memory, threads, etc.)
    #[error("Resource exhausted: {resource}: {reason}")]
    ResourceExhausted {
        /// Type of resource that was exhausted
        resource: String,
        /// Reason for exhaustion
        reason: String,
    },

    /// Invalid audio data or parameters
    #[error("Invalid audio data: {field}: {reason}")]
    InvalidData {
        /// Field or parameter that is invalid
        field: String,
        /// Reason why it's invalid
        reason: String,
    },

    /// Feature not implemented or not available
    #[error("Feature not available: {feature}: {reason}")]
    FeatureNotAvailable {
        /// Feature name
        feature: String,
        /// Reason why feature is not available
        reason: String,
    },

    /// Internal library error
    #[error("Internal audio-core error: {message}")]
    Internal {
        /// Internal error message
        message: String,
    },
}

impl AudioError {
    /// Create a device not found error
    pub fn device_not_found<S: Into<String>>(device_id: S) -> Self {
        Self::DeviceNotFound {
            device_id: device_id.into(),
        }
    }

    /// Create a device in use error
    pub fn device_in_use<S: Into<String>>(device_id: S) -> Self {
        Self::DeviceInUse {
            device_id: device_id.into(),
        }
    }

    /// Create a format not supported error
    pub fn format_not_supported<S1: Into<String>, S2: Into<String>>(format: S1, component: S2) -> Self {
        Self::FormatNotSupported {
            format: format.into(),
            component: component.into(),
        }
    }


    /// Create a buffer error
    pub fn buffer_error<S1: Into<String>, S2: Into<String>, S3: Into<String>>(
        buffer_type: S1,
        error_type: S2,
        details: S3,
    ) -> Self {
        Self::BufferError {
            buffer_type: buffer_type.into(),
            error_type: error_type.into(),
            details: details.into(),
        }
    }

    /// Create a pipeline error
    pub fn pipeline_error<S1: Into<String>, S2: Into<String>>(stage: S1, reason: S2) -> Self {
        Self::PipelineError {
            stage: stage.into(),
            reason: reason.into(),
        }
    }

    /// Create a platform error
    pub fn platform_error<S: Into<String>>(message: S) -> Self {
        Self::PlatformError {
            message: message.into(),
        }
    }

    /// Create a configuration error
    pub fn configuration_error<S1: Into<String>, S2: Into<String>>(component: S1, reason: S2) -> Self {
        Self::ConfigurationError {
            component: component.into(),
            reason: reason.into(),
        }
    }

    /// Create an internal error
    pub fn internal<S: Into<String>>(message: S) -> Self {
        Self::Internal {
            message: message.into(),
        }
    }

    /// Create an invalid configuration error (convenience method)
    pub fn invalid_configuration<S: Into<String>>(reason: S) -> Self {
        Self::ConfigurationError {
            component: "configuration".to_string(),
            reason: reason.into(),
        }
    }

    /// Create an invalid format error (convenience method)
    pub fn invalid_format<S: Into<String>>(reason: S) -> Self {
        Self::InvalidData {
            field: "format".to_string(),
            reason: reason.into(),
        }
    }

    /// Check if this error is recoverable
    pub fn is_recoverable(&self) -> bool {
        match self {
            // Recoverable errors - might work if retried
            Self::DeviceInUse { .. } => true,
            Self::BufferError { .. } => true,
            Self::Timeout { .. } => true,
            Self::IoError { .. } => true,
            Self::ResourceExhausted { .. } => true,
            
            // Non-recoverable errors - fundamental issues
            Self::DeviceNotFound { .. } => false,
            Self::FormatNotSupported { .. } => false,
            Self::ConfigurationError { .. } => false,
            Self::InvalidData { .. } => false,
            Self::FeatureNotAvailable { .. } => false,
            Self::Internal { .. } => false,
            
            // Conditionally recoverable
            Self::DeviceDisconnected { .. } => true, // Device might reconnect
            Self::FormatConversionFailed { .. } => true, // Different parameters might work
            Self::PipelineError { .. } => true, // Pipeline can be restarted
            Self::RtpPayloadError { .. } => true, // Next packet might be OK
            Self::SynchronizationError { .. } => true, // Can resynchronize
            Self::PlatformError { .. } => false, // Platform issues are usually fundamental
            Self::SessionError { .. } => true, // Session can be reestablished
        }
    }

    /// Get a user-friendly error message
    pub fn user_friendly_message(&self) -> String {
        match self {
            Self::DeviceNotFound { device_id } => {
                format!("Audio device '{}' not found or not available. Please check your audio settings.", device_id)
            }
            Self::DeviceInUse { device_id } => {
                format!("Audio device '{}' is currently being used by another application.", device_id)
            }
            Self::DeviceDisconnected { device_id } => {
                format!("Audio device '{}' was disconnected. Please reconnect the device.", device_id)
            }
            Self::FormatNotSupported { format, .. } => {
                format!("The audio format '{}' is not supported. Try a different audio setting.", format)
            }
            Self::BufferError { .. } => {
                "Audio buffer issue detected. This may cause audio glitches.".to_string()
            }
            Self::PlatformError { .. } => {
                "An audio system error occurred. Please check your audio drivers and settings.".to_string()
            }
            _ => "An audio error occurred. Please try again.".to_string(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_error_creation() {
        let err = AudioError::device_not_found("test-device");
        assert!(matches!(err, AudioError::DeviceNotFound { .. }));
    }

    #[test]
    fn test_error_recoverability() {
        let recoverable = AudioError::device_in_use("test");
        assert!(recoverable.is_recoverable());

        let non_recoverable = AudioError::device_not_found("test");
        assert!(!non_recoverable.is_recoverable());
    }

    #[test]
    fn test_user_friendly_messages() {
        let err = AudioError::device_not_found("speakers");
        let msg = err.user_friendly_message();
        assert!(msg.contains("not available"));
        assert!(msg.contains("speakers"));
    }
} 