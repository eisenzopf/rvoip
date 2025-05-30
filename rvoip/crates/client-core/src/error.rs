//! Error types for client-core operations

use thiserror::Error;
use uuid::Uuid;

/// Result type alias for client-core operations
pub type ClientResult<T> = Result<T, ClientError>;

/// Comprehensive error types for SIP client operations
#[derive(Error, Debug)]
pub enum ClientError {
    /// Registration related errors
    #[error("Registration failed: {reason}")]
    RegistrationFailed { reason: String },

    #[error("Not registered with server")]
    NotRegistered,

    #[error("Registration expired")]
    RegistrationExpired,

    #[error("Authentication failed: {reason}")]
    AuthenticationFailed { reason: String },

    /// Call related errors
    #[error("Call not found: {call_id}")]
    CallNotFound { call_id: Uuid },

    #[error("Call already exists: {call_id}")]
    CallAlreadyExists { call_id: Uuid },

    #[error("Invalid call state for call {call_id}: current state is {current_state:?}")]
    InvalidCallState { 
        call_id: Uuid, 
        current_state: crate::call::CallState 
    },

    #[error("Invalid call state: expected {expected}, got {actual}")]
    InvalidCallStateGeneric { expected: String, actual: String },

    #[error("Call setup failed: {reason}")]
    CallSetupFailed { reason: String },

    #[error("Call terminated: {reason}")]
    CallTerminated { reason: String },

    /// Media related errors
    #[error("Media negotiation failed: {reason}")]
    MediaNegotiationFailed { reason: String },

    #[error("No compatible codecs")]
    NoCompatibleCodecs,

    #[error("Audio device error: {reason}")]
    AudioDeviceError { reason: String },

    /// Network and transport errors
    #[error("Network error: {reason}")]
    NetworkError { reason: String },

    #[error("Connection timeout")]
    ConnectionTimeout,

    #[error("Server unreachable: {server}")]
    ServerUnreachable { server: String },

    /// Protocol errors
    #[error("SIP protocol error: {reason}")]
    ProtocolError { reason: String },

    #[error("Invalid SIP URI: {uri}")]
    InvalidUri { uri: String },

    #[error("Unsupported SIP method: {method}")]
    UnsupportedMethod { method: String },

    /// Configuration errors
    #[error("Invalid configuration: {reason}")]
    InvalidConfiguration { reason: String },

    #[error("Missing required configuration: {field}")]
    MissingConfiguration { field: String },

    /// Infrastructure errors (wrapping lower-layer errors)
    #[error("Transaction error: {0}")]
    TransactionError(#[from] anyhow::Error),

    #[error("Media error: {0}")]
    MediaError(String),

    #[error("Transport error: {0}")]
    TransportError(String),

    /// General errors
    #[error("Internal error: {reason}")]
    InternalError { reason: String },

    #[error("Operation timeout")]
    Timeout,

    #[error("Operation cancelled")]
    Cancelled,
}

impl ClientError {
    /// Create a registration failed error
    pub fn registration_failed(reason: impl Into<String>) -> Self {
        Self::RegistrationFailed { reason: reason.into() }
    }

    /// Create an authentication failed error
    pub fn authentication_failed(reason: impl Into<String>) -> Self {
        Self::AuthenticationFailed { reason: reason.into() }
    }

    /// Create a call setup failed error
    pub fn call_setup_failed(reason: impl Into<String>) -> Self {
        Self::CallSetupFailed { reason: reason.into() }
    }

    /// Create a media negotiation failed error
    pub fn media_negotiation_failed(reason: impl Into<String>) -> Self {
        Self::MediaNegotiationFailed { reason: reason.into() }
    }

    /// Create a network error
    pub fn network_error(reason: impl Into<String>) -> Self {
        Self::NetworkError { reason: reason.into() }
    }

    /// Create a protocol error
    pub fn protocol_error(reason: impl Into<String>) -> Self {
        Self::ProtocolError { reason: reason.into() }
    }

    /// Create an internal error
    pub fn internal_error(reason: impl Into<String>) -> Self {
        Self::InternalError { reason: reason.into() }
    }

    /// Check if error is recoverable (can retry operation)
    pub fn is_recoverable(&self) -> bool {
        matches!(
            self,
            ClientError::ConnectionTimeout
                | ClientError::NetworkError { .. }
                | ClientError::ServerUnreachable { .. }
                | ClientError::Timeout
                | ClientError::TransportError(..)
        )
    }

    /// Check if error indicates authentication issue
    pub fn is_auth_error(&self) -> bool {
        matches!(
            self,
            ClientError::AuthenticationFailed { .. }
                | ClientError::NotRegistered
                | ClientError::RegistrationExpired
        )
    }

    /// Check if error is call-related
    pub fn is_call_error(&self) -> bool {
        matches!(
            self,
            ClientError::CallNotFound { .. }
                | ClientError::CallAlreadyExists { .. }
                | ClientError::InvalidCallState { .. }
                | ClientError::CallSetupFailed { .. }
                | ClientError::CallTerminated { .. }
        )
    }
} 