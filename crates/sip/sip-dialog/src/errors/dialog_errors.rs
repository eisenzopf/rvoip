//! Dialog-specific error types
//!
//! This module defines error types for dialog operations including
//! dialog creation, state management, request routing, and protocol handling.

use std::time::SystemTime;
use std::{error, fmt};

/// Result type for dialog operations
pub type DialogResult<T> = Result<T, DialogError>;

/// Main error type for dialog operations
#[derive(Clone)]
pub enum DialogError {
    /// Dialog not found
    DialogNotFound { id: String },

    /// Invalid dialog state for operation
    InvalidState { expected: String, actual: String },

    /// Dialog already exists
    DialogAlreadyExists { id: String },

    /// Transaction error from transaction-core
    TransactionError { message: String },

    /// SIP protocol error
    ProtocolError { message: String },

    /// Request routing error
    RoutingError { message: String },

    /// SDP negotiation error
    SdpError { message: String },

    /// Internal error with context
    InternalError {
        message: String,
        context: Option<ErrorContext>,
    },

    /// Network/connectivity error
    NetworkError { message: String },

    /// Timeout error
    TimeoutError { operation: String },

    /// Configuration error
    ConfigError { message: String },
}

impl DialogError {
    pub const fn diagnostic_class(&self) -> &'static str {
        match self {
            Self::DialogNotFound { .. } => "dialog-not-found",
            Self::InvalidState { .. } => "invalid-state",
            Self::DialogAlreadyExists { .. } => "dialog-already-exists",
            Self::TransactionError { .. } => "transaction",
            Self::ProtocolError { .. } => "protocol",
            Self::RoutingError { .. } => "routing",
            Self::SdpError { .. } => "sdp",
            Self::InternalError { .. } => "internal",
            Self::NetworkError { .. } => "network",
            Self::TimeoutError { .. } => "timeout",
            Self::ConfigError { .. } => "configuration",
        }
    }
}

#[cfg(test)]
mod diagnostic_tests {
    use super::*;

    #[test]
    fn every_dialog_error_variant_is_payload_free() {
        const CANARY: &str = "dialog-error-direct-secret-canary";
        let context = ErrorContext {
            dialog_id: Some(CANARY.into()),
            transaction_id: Some(CANARY.into()),
            call_id: Some(CANARY.into()),
            timestamp: SystemTime::UNIX_EPOCH,
            details: Some(CANARY.into()),
        };
        let errors = vec![
            DialogError::DialogNotFound { id: CANARY.into() },
            DialogError::InvalidState {
                expected: CANARY.into(),
                actual: CANARY.into(),
            },
            DialogError::DialogAlreadyExists { id: CANARY.into() },
            DialogError::TransactionError {
                message: CANARY.into(),
            },
            DialogError::ProtocolError {
                message: CANARY.into(),
            },
            DialogError::RoutingError {
                message: CANARY.into(),
            },
            DialogError::SdpError {
                message: CANARY.into(),
            },
            DialogError::InternalError {
                message: CANARY.into(),
                context: Some(context.clone()),
            },
            DialogError::NetworkError {
                message: CANARY.into(),
            },
            DialogError::TimeoutError {
                operation: CANARY.into(),
            },
            DialogError::ConfigError {
                message: CANARY.into(),
            },
        ];

        for error in errors {
            let rendered = format!("{error:?} {error}");
            assert!(!rendered.contains(CANARY), "payload leaked: {rendered}");
            assert!(!error.diagnostic_class().is_empty());
            assert!(std::error::Error::source(&error).is_none());
        }
        assert!(!format!("{context:?}").contains(CANARY));
    }
}

impl fmt::Display for DialogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(
            formatter,
            "SIP dialog operation failed (class={})",
            self.diagnostic_class()
        )
    }
}

impl fmt::Debug for DialogError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("DialogError")
            .field("class", &self.diagnostic_class())
            .field(
                "context_present",
                &matches!(
                    self,
                    Self::InternalError {
                        context: Some(_),
                        ..
                    }
                ),
            )
            .finish()
    }
}

impl error::Error for DialogError {}

/// Additional context for errors
#[derive(Clone)]
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

impl fmt::Debug for ErrorContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("ErrorContext")
            .field("dialog_id_present", &self.dialog_id.is_some())
            .field("transaction_id_present", &self.transaction_id.is_some())
            .field("call_id_present", &self.call_id.is_some())
            .field("timestamp", &self.timestamp)
            .field("details_present", &self.details.is_some())
            .finish()
    }
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
        Self::DialogNotFound { id: id.to_string() }
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
