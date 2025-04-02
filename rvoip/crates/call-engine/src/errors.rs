use std::fmt;
use thiserror::Error;

/// Errors that can occur in the call engine
#[derive(Error, Debug)]
pub enum Error {
    /// Session-related errors
    #[error("Session error: {0}")]
    Session(#[from] rvoip_session_core::Error),
    
    /// SIP-related errors
    #[error("SIP error: {0}")]
    Sip(#[from] rvoip_sip_core::Error),
    
    /// Transaction-related errors
    #[error("Transaction error: {0}")]
    Transaction(#[from] rvoip_transaction_core::Error),
    
    /// Media-related errors
    #[error("Media error: {0}")]
    Media(String),
    
    /// Routing errors
    #[error("Routing error: {0}")]
    Routing(String),
    
    /// User not found
    #[error("User not found: {0}")]
    UserNotFound(String),
    
    /// Endpoint not found
    #[error("Endpoint not found: {0}")]
    EndpointNotFound(String),
    
    /// Authentication failed
    #[error("Authentication failed: {0}")]
    AuthenticationFailed(String),
    
    /// Authorization failed
    #[error("Authorization failed: {0}")]
    AuthorizationFailed(String),
    
    /// Resource not available
    #[error("Resource not available: {0}")]
    ResourceNotAvailable(String),
    
    /// Policy violation
    #[error("Policy violation: {0}")]
    PolicyViolation(String),
    
    /// Other errors
    #[error("{0}")]
    Other(String),
}

impl Error {
    /// Create a new Other error with a string message
    pub fn other<S: fmt::Display>(msg: S) -> Self {
        Self::Other(msg.to_string())
    }
    
    /// Create a new Routing error with a string message
    pub fn routing<S: fmt::Display>(msg: S) -> Self {
        Self::Routing(msg.to_string())
    }
    
    /// Create a new Media error with a string message
    pub fn media<S: fmt::Display>(msg: S) -> Self {
        Self::Media(msg.to_string())
    }
} 