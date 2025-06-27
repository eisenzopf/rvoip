//! Error types for RVOIP Simple

use thiserror::Error;

/// Main error type for Simple VoIP operations
#[derive(Error, Debug)]
pub enum SimpleVoipError {
    /// Network connection errors
    #[error("Network error: {0}")]
    Network(String),

    /// SIP protocol errors
    #[error("SIP error: {0}")]
    Sip(String),

    /// RTP/Media errors
    #[error("Media error: {0}")]
    Media(String),

    /// Authentication/Security errors
    #[error("Security error: {0}")]
    Security(String),

    /// Configuration errors
    #[error("Configuration error: {0}")]
    Configuration(String),

    /// Call setup/management errors
    #[error("Call error: {0}")]
    Call(String),

    /// Timeout errors
    #[error("Operation timed out: {0}")]
    Timeout(String),

    /// Invalid state for operation
    #[error("Invalid state: {0}")]
    InvalidState(String),

    /// Not implemented features
    #[error("Feature not implemented: {0}")]
    NotImplemented(String),

    /// Generic errors with context
    #[error("Error: {0}")]
    Generic(String),
}

impl SimpleVoipError {
    /// Create a network error
    pub fn network(msg: impl Into<String>) -> Self {
        Self::Network(msg.into())
    }

    /// Create a SIP error
    pub fn sip(msg: impl Into<String>) -> Self {
        Self::Sip(msg.into())
    }

    /// Create a media error
    pub fn media(msg: impl Into<String>) -> Self {
        Self::Media(msg.into())
    }

    /// Create a security error
    pub fn security(msg: impl Into<String>) -> Self {
        Self::Security(msg.into())
    }

    /// Create a configuration error
    pub fn configuration(msg: impl Into<String>) -> Self {
        Self::Configuration(msg.into())
    }

    /// Create a call error
    pub fn call(msg: impl Into<String>) -> Self {
        Self::Call(msg.into())
    }

    /// Create a timeout error
    pub fn timeout(msg: impl Into<String>) -> Self {
        Self::Timeout(msg.into())
    }

    /// Create an invalid state error
    pub fn invalid_state(msg: impl Into<String>) -> Self {
        Self::InvalidState(msg.into())
    }

    /// Create a not implemented error
    pub fn not_implemented(feature: impl Into<String>) -> Self {
        Self::NotImplemented(feature.into())
    }

    /// Check if this is a recoverable error
    pub fn is_recoverable(&self) -> bool {
        matches!(self, 
            SimpleVoipError::Network(_) | 
            SimpleVoipError::Timeout(_) |
            SimpleVoipError::InvalidState(_)
        )
    }

    /// Check if this is a configuration-related error
    pub fn is_configuration_error(&self) -> bool {
        matches!(self, 
            SimpleVoipError::Configuration(_) |
            SimpleVoipError::Security(_)
        )
    }
}

/// Result type for Simple VoIP operations
pub type SimpleVoipResult<T> = Result<T, SimpleVoipError>;

/// Convert common error types to SimpleVoipError
impl From<std::io::Error> for SimpleVoipError {
    fn from(err: std::io::Error) -> Self {
        Self::Network(err.to_string())
    }
}

impl From<tokio::time::error::Elapsed> for SimpleVoipError {
    fn from(err: tokio::time::error::Elapsed) -> Self {
        Self::Timeout(err.to_string())
    }
} 