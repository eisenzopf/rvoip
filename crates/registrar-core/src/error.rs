//! Error types for registrar-core

use thiserror::Error;

/// Result type alias for registrar operations
pub type Result<T> = std::result::Result<T, RegistrarError>;

/// Main error type for registrar operations
#[derive(Error, Debug)]
pub enum RegistrarError {
    /// User not found in registry
    #[error("User not found: {0}")]
    UserNotFound(String),
    
    /// Contact not found for user
    #[error("Contact not found: {uri} for user {user}")]
    ContactNotFound { user: String, uri: String },
    
    /// Registration has expired
    #[error("Registration expired for user: {0}")]
    RegistrationExpired(String),
    
    /// Invalid registration parameters
    #[error("Invalid registration: {0}")]
    InvalidRegistration(String),
    
    /// Maximum contacts exceeded
    #[error("Maximum contacts ({max}) exceeded for user: {user}")]
    MaxContactsExceeded { user: String, max: usize },
    
    /// Subscription not found
    #[error("Subscription not found: {0}")]
    SubscriptionNotFound(String),
    
    /// Invalid subscription
    #[error("Invalid subscription: {0}")]
    InvalidSubscription(String),
    
    /// Maximum subscriptions exceeded
    #[error("Maximum subscriptions ({max}) exceeded for user: {user}")]
    MaxSubscriptionsExceeded { user: String, max: usize },
    
    /// Presence not found
    #[error("Presence not found for user: {0}")]
    PresenceNotFound(String),
    
    /// Invalid presence data
    #[error("Invalid presence data: {0}")]
    InvalidPresence(String),
    
    /// PIDF XML parsing error
    #[error("PIDF parsing error: {0}")]
    PidfError(String),
    
    /// Event bus error
    #[error("Event bus error: {0}")]
    EventBusError(String),
    
    /// Configuration error
    #[error("Configuration error: {0}")]
    ConfigError(String),
    
    /// Storage error (for future persistent storage)
    #[error("Storage error: {0}")]
    StorageError(String),
    
    /// Timeout error
    #[error("Operation timed out: {0}")]
    Timeout(String),
    
    /// Internal error
    #[error("Internal error: {0}")]
    Internal(String),
    
    /// Other errors
    #[error("{0}")]
    Other(String),
}

impl From<std::io::Error> for RegistrarError {
    fn from(err: std::io::Error) -> Self {
        RegistrarError::Internal(err.to_string())
    }
}

impl From<serde_json::Error> for RegistrarError {
    fn from(err: serde_json::Error) -> Self {
        RegistrarError::Internal(format!("JSON error: {}", err))
    }
}

impl From<quick_xml::Error> for RegistrarError {
    fn from(err: quick_xml::Error) -> Self {
        RegistrarError::PidfError(err.to_string())
    }
}