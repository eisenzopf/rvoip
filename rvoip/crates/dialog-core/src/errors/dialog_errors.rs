//! Dialog-specific error types
//!
//! This module defines error types for dialog operations including
//! dialog creation, state management, request routing, and protocol handling.

use std::time::SystemTime;
use thiserror::Error;

/// Result type for dialog operations
pub type DialogResult<T> = Result<T, DialogError>;

/// Main error type for dialog operations
#[derive(Error, Debug, Clone)]
pub enum DialogError {
    /// Dialog not found
    #[error("Dialog not found: {id}")]
    DialogNotFound { id: String },
    
    /// Invalid dialog state for operation
    #[error("Invalid dialog state: expected {expected}, got {actual}")]
    InvalidState { expected: String, actual: String },
    
    /// Dialog already exists
    #[error("Dialog already exists: {id}")]
    DialogAlreadyExists { id: String },
    
    /// Transaction error from transaction-core
    #[error("Transaction error: {message}")]
    TransactionError { message: String },
    
    /// SIP protocol error
    #[error("SIP protocol error: {message}")]
    ProtocolError { message: String },
    
    /// Request routing error
    #[error("Request routing error: {message}")]
    RoutingError { message: String },
    
    /// SDP negotiation error
    #[error("SDP negotiation error: {message}")]
    SdpError { message: String },
    
    /// Internal error with context
    #[error("Internal error: {message}")]
    InternalError { 
        message: String,
        context: Option<ErrorContext>,
    },
    
    /// Network/connectivity error
    #[error("Network error: {message}")]
    NetworkError { message: String },
    
    /// Timeout error
    #[error("Operation timed out: {operation}")]
    TimeoutError { operation: String },
    
    /// Configuration error
    #[error("Configuration error: {message}")]
    ConfigError { message: String },
}

/// Additional context for errors
#[derive(Debug, Clone)]
pub struct ErrorContext {
    /// Dialog ID if applicable
    pub dialog_id: Option<String>,
    
    /// Transaction ID if applicable
    pub transaction_id: Option<String>,
    
    /// Call-ID if applicable
    pub call_id: Option<String>,
    
    /// Timestamp when error occurred
    pub timestamp: SystemTime,
    
    /// Additional details
    pub details: Option<String>,
}

impl Default for ErrorContext {
    fn default() -> Self {
        Self {
            dialog_id: None,
            transaction_id: None,
            call_id: None,
            timestamp: SystemTime::now(),
            details: None,
        }
    }
}

impl ErrorContext {
    /// Create a new error context with a dialog ID
    pub fn with_dialog_id(dialog_id: String) -> Self {
        Self {
            dialog_id: Some(dialog_id),
            ..Default::default()
        }
    }
    
    /// Create a new error context with a transaction ID
    pub fn with_transaction_id(transaction_id: String) -> Self {
        Self {
            transaction_id: Some(transaction_id),
            ..Default::default()
        }
    }
    
    /// Add details to the context
    pub fn with_details(mut self, details: String) -> Self {
        self.details = Some(details);
        self
    }
}

// Convenience constructors for common errors
impl DialogError {
    /// Create a dialog not found error
    pub fn dialog_not_found(id: &str) -> Self {
        Self::DialogNotFound {
            id: id.to_string(),
        }
    }
    
    /// Create an invalid state error
    pub fn invalid_state(expected: &str, actual: &str) -> Self {
        Self::InvalidState {
            expected: expected.to_string(),
            actual: actual.to_string(),
        }
    }
    
    /// Create a protocol error
    pub fn protocol_error(message: &str) -> Self {
        Self::ProtocolError {
            message: message.to_string(),
        }
    }
    
    /// Create a routing error
    pub fn routing_error(message: &str) -> Self {
        Self::RoutingError {
            message: message.to_string(),
        }
    }
    
    /// Create an internal error with context
    pub fn internal_error(message: &str, context: Option<ErrorContext>) -> Self {
        Self::InternalError {
            message: message.to_string(),
            context,
        }
    }
} 