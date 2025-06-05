//! Error Types for Session Core
//!
//! Simplified error handling with the main error types needed for session management.

use std::fmt;

/// Main result type for session operations
pub type Result<T> = std::result::Result<T, SessionError>;

/// Main error type for session operations
#[derive(Debug, Clone)]
pub enum SessionError {
    /// Invalid session state for the requested operation
    InvalidState(String),
    
    /// Session not found
    SessionNotFound(String),
    
    /// Media-related error
    MediaError(String),
    
    /// SIP protocol error
    SipError(String),
    
    /// Network/transport error
    NetworkError(String),
    
    /// Configuration error
    ConfigError(String),
    
    /// Resource limit exceeded
    ResourceLimitExceeded(String),
    
    /// Timeout error
    Timeout(String),
    
    /// Generic error with message
    Other(String),
}

impl fmt::Display for SessionError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            SessionError::InvalidState(msg) => write!(f, "Invalid state: {}", msg),
            SessionError::SessionNotFound(msg) => write!(f, "Session not found: {}", msg),
            SessionError::MediaError(msg) => write!(f, "Media error: {}", msg),
            SessionError::SipError(msg) => write!(f, "SIP error: {}", msg),
            SessionError::NetworkError(msg) => write!(f, "Network error: {}", msg),
            SessionError::ConfigError(msg) => write!(f, "Configuration error: {}", msg),
            SessionError::ResourceLimitExceeded(msg) => write!(f, "Resource limit exceeded: {}", msg),
            SessionError::Timeout(msg) => write!(f, "Timeout: {}", msg),
            SessionError::Other(msg) => write!(f, "Error: {}", msg),
        }
    }
}

impl std::error::Error for SessionError {}

impl From<String> for SessionError {
    fn from(msg: String) -> Self {
        SessionError::Other(msg)
    }
}

impl From<&str> for SessionError {
    fn from(msg: &str) -> Self {
        SessionError::Other(msg.to_string())
    }
}

impl From<std::io::Error> for SessionError {
    fn from(err: std::io::Error) -> Self {
        SessionError::NetworkError(err.to_string())
    }
}

// Convenience constructors
impl SessionError {
    pub fn invalid_state(msg: &str) -> Self {
        SessionError::InvalidState(msg.to_string())
    }

    pub fn session_not_found(session_id: &str) -> Self {
        SessionError::SessionNotFound(session_id.to_string())
    }

    pub fn media_error(msg: &str) -> Self {
        SessionError::MediaError(msg.to_string())
    }

    pub fn sip_error(msg: &str) -> Self {
        SessionError::SipError(msg.to_string())
    }

    pub fn network_error(msg: &str) -> Self {
        SessionError::NetworkError(msg.to_string())
    }

    pub fn config_error(msg: &str) -> Self {
        SessionError::ConfigError(msg.to_string())
    }

    pub fn timeout(msg: &str) -> Self {
        SessionError::Timeout(msg.to_string())
    }

    pub fn internal(msg: &str) -> Self {
        SessionError::Other(msg.to_string())
    }
} 