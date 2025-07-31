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
}

impl SipClientError {
    /// Create a configuration error
    pub fn config(message: impl Into<String>) -> Self {
        Self::Configuration {
            message: message.into(),
        }
    }
    
    /// Create an invalid state error
    pub fn invalid_state(message: impl Into<String>) -> Self {
        Self::InvalidState {
            message: message.into(),
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
}