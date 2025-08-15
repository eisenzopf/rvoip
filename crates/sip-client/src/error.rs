//! Error types for the SIP client library

use thiserror::Error;

/// Result type for SIP client operations
pub type SipClientResult<T> = Result<T, SipClientError>;

/// Errors that can occur in the SIP client
#[derive(Debug, Error)]
pub enum SipClientError {
    /// Client-core error
    #[error("SIP client error: {0}")]
    ClientCore(#[from] rvoip_client_core::ClientError),
    
    /// Audio-core error
    #[error("Audio error: {0}")]
    AudioCore(#[from] rvoip_audio_core::AudioError),
    
    /// Codec-core error
    #[error("Codec error: {0}")]
    CodecCore(#[from] codec_core::CodecError),
    
    /// Configuration error
    #[error("Configuration error: {message}")]
    Configuration { message: String },
    
    /// Invalid state error
    #[error("Invalid state: {message}")]
    InvalidState { message: String },
    
    /// Call not found
    #[error("Call not found: {call_id}")]
    CallNotFound { call_id: String },
    
    /// Audio device error
    #[error("Audio device error: {message}")]
    AudioDevice { message: String },
    
    /// Audio pipeline error
    #[error("Audio pipeline error in {operation}: {details}")]
    AudioPipelineError { 
        operation: String,
        details: String,
    },
    
    /// Codec negotiation failed
    #[error("Codec negotiation failed: {reason}")]
    CodecNegotiationFailed { reason: String },
    
    /// Network error
    #[error("Network error: {message}")]
    Network { message: String },
    
    /// Timeout error
    #[error("Operation timed out after {seconds} seconds")]
    Timeout { seconds: u64 },
    
    /// Not implemented
    #[error("Feature not implemented: {feature}")]
    NotImplemented { feature: String },
    
    /// Internal error
    #[error("Internal error: {message}")]
    Internal { message: String },
    
    /// Registration failed
    #[error("Registration failed: {reason}")]
    RegistrationFailed { reason: String },
    
    /// Call failed
    #[error("Call {call_id} failed: {reason}")]
    CallFailed {
        call_id: String,
        reason: String,
    },
    
    /// Codec error with details
    #[error("Codec '{codec}' error: {details}")]
    CodecError {
        codec: String,
        details: String,
    },
    
    /// Invalid input
    #[error("Invalid input for {field}: {reason}")]
    InvalidInput {
        field: String,
        reason: String,
    },
    
    /// Transfer failed
    #[error("Transfer failed: {reason}")]
    TransferFailed { reason: String },
}

// Note: Clone is not implemented because some wrapped errors don't implement Clone.
// If you need to clone an error, consider converting it to a string first.

impl SipClientError {
    /// Create a configuration error
    pub fn config(message: impl Into<String>) -> Self {
        Self::Configuration {
            message: message.into(),
        }
    }
    
    /// Create an invalid state error with expected and actual state
    pub fn invalid_state(message: impl Into<String>) -> Self {
        Self::InvalidState {
            message: message.into(),
        }
    }
    
    /// Create an invalid state error with expected and actual values
    pub fn invalid_state_with_details(expected: &str, actual: &str) -> Self {
        Self::InvalidState {
            message: format!("Expected state: {}, but was: {}", expected, actual),
        }
    }
    
    /// Create an audio device error
    pub fn audio_device(message: impl Into<String>) -> Self {
        Self::AudioDevice {
            message: message.into(),
        }
    }
    
    /// Create a codec negotiation error
    pub fn codec_negotiation(reason: impl Into<String>) -> Self {
        Self::CodecNegotiationFailed {
            reason: reason.into(),
        }
    }
    
    /// Create a network error
    pub fn network(message: impl Into<String>) -> Self {
        Self::Network {
            message: message.into(),
        }
    }
    
    /// Create an internal error
    pub fn internal(message: impl Into<String>) -> Self {
        Self::Internal {
            message: message.into(),
        }
    }
    
    /// Create an invalid input error
    pub fn invalid_input(field: impl Into<String>, reason: impl Into<String>) -> Self {
        Self::InvalidInput {
            field: field.into(),
            reason: reason.into(),
        }
    }
}