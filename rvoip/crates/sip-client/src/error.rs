//! Error handling - Simple, clean error types for the SIP client
//!
//! This module provides straightforward error handling with good error messages
//! and appropriate error classification.

use thiserror::Error;

/// Result type alias for all SIP client operations
pub type Result<T> = std::result::Result<T, Error>;

/// Main error type for the SIP client
#[derive(Error, Debug)]
pub enum Error {
    /// Configuration errors
    #[error("Configuration error: {0}")]
    Configuration(String),

    /// Network and connection errors
    #[error("Network error: {0}")]
    Network(String),

    /// SIP protocol errors
    #[error("SIP protocol error: {0}")]
    Protocol(String),

    /// Call-related errors
    #[error("Call error: {0}")]
    Call(String),

    /// Registration errors
    #[error("Registration error: {0}")]
    Registration(String),

    /// Media errors (audio/video)
    #[error("Media error: {0}")]
    Media(String),

    /// Authentication errors
    #[error("Authentication error: {0}")]
    Authentication(String),

    /// Errors from the core client infrastructure
    #[error("Core client error: {0}")]
    Core(String),

    /// Call-engine integration errors
    #[error("Call-engine error: {0}")]
    CallEngine(String),

    /// Timeout errors
    #[error("Operation timed out: {0}")]
    Timeout(String),

    /// I/O errors (file operations, etc.)
    #[error("I/O error: {0}")]
    Io(#[from] std::io::Error),

    /// Generic errors
    #[error("Error: {0}")]
    Other(String),
}

impl Error {
    /// Check if this error is recoverable (operation can be retried)
    pub fn is_recoverable(&self) -> bool {
        matches!(
            self,
            Error::Network(_) | Error::Timeout(_) | Error::Io(_)
        )
    }

    /// Check if this error is related to authentication
    pub fn is_auth_error(&self) -> bool {
        matches!(self, Error::Authentication(_) | Error::Registration(_))
    }

    /// Check if this error is related to call handling
    pub fn is_call_error(&self) -> bool {
        matches!(self, Error::Call(_) | Error::Media(_))
    }

    /// Get a user-friendly error message
    pub fn user_message(&self) -> String {
        match self {
            Error::Configuration(msg) => format!("Configuration problem: {}", msg),
            Error::Network(msg) => format!("Network problem: {}", msg),
            Error::Protocol(msg) => format!("SIP protocol issue: {}", msg),
            Error::Call(msg) => format!("Call issue: {}", msg),
            Error::Registration(msg) => format!("Registration problem: {}", msg),
            Error::Media(msg) => format!("Audio/media issue: {}", msg),
            Error::Authentication(msg) => format!("Authentication failed: {}", msg),
            Error::Core(msg) => format!("Internal error: {}", msg),
            Error::CallEngine(msg) => format!("Call center issue: {}", msg),
            Error::Timeout(msg) => format!("Timed out: {}", msg),
            Error::Io(e) => format!("File operation failed: {}", e),
            Error::Other(msg) => msg.clone(),
        }
    }

    /// Convert from anyhow::Error (for compatibility)
    pub fn from_anyhow(err: anyhow::Error) -> Self {
        Error::Other(err.to_string())
    }
} 