//! Error types for client-core operations

use thiserror::Error;
use uuid::Uuid;

/// Result type alias for client-core operations
pub type ClientResult<T> = Result<T, ClientError>;

/// Comprehensive error types for SIP client operations
#[derive(Error, Debug, Clone)]
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

    #[error("Invalid SIP message: {reason}")]
    InvalidSipMessage { reason: String },

    #[error("Protocol version mismatch: expected {expected}, got {actual}")]
    ProtocolVersionMismatch { expected: String, actual: String },

    /// Configuration errors
    #[error("Invalid configuration: {field} - {reason}")]
    InvalidConfiguration { field: String, reason: String },

    #[error("Missing required configuration: {field}")]
    MissingConfiguration { field: String },

    /// Transport errors
    #[error("Transport failed: {reason}")]
    TransportFailed { reason: String },

    #[error("Transport not available: {transport_type}")]
    TransportNotAvailable { transport_type: String },

    /// Session management errors
    #[error("Session manager error: {reason}")]
    SessionManagerError { reason: String },

    #[error("Too many sessions: limit is {limit}")]
    TooManySessions { limit: usize },

    /// Generic errors
    #[error("Internal error: {message}")]
    InternalError { message: String },

    #[error("Operation timeout after {duration_ms}ms")]
    OperationTimeout { duration_ms: u64 },

    #[error("Not implemented: {feature} - {reason}")]
    NotImplemented { feature: String, reason: String },

    #[error("Permission denied: {operation}")]
    PermissionDenied { operation: String },

    #[error("Resource unavailable: {resource}")]
    ResourceUnavailable { resource: String },

    /// Codec and media format errors
    #[error("Unsupported codec: {codec}")]
    UnsupportedCodec { codec: String },

    #[error("Codec error: {reason}")]
    CodecError { reason: String },

    /// External service errors
    #[error("External service error: {service} - {reason}")]
    ExternalServiceError { service: String, reason: String },
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
        Self::InternalError { message: reason.into() }
    }

    /// Check if this error is recoverable
    pub fn is_recoverable(&self) -> bool {
        match self {
            // Recoverable errors
            ClientError::NetworkError { .. } |
            ClientError::ConnectionTimeout |
            ClientError::TransportFailed { .. } |
            ClientError::OperationTimeout { .. } |
            ClientError::ExternalServiceError { .. } => true,
            
            // Non-recoverable errors
            ClientError::InvalidConfiguration { .. } |
            ClientError::MissingConfiguration { .. } |
            ClientError::ProtocolVersionMismatch { .. } |
            ClientError::PermissionDenied { .. } |
            ClientError::NotImplemented { .. } |
            ClientError::UnsupportedCodec { .. } => false,
            
            // Context-dependent errors
            _ => false,
        }
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

    /// Get error category for metrics/logging
    pub fn category(&self) -> &'static str {
        match self {
            ClientError::RegistrationFailed { .. } |
            ClientError::NotRegistered |
            ClientError::RegistrationExpired |
            ClientError::AuthenticationFailed { .. } => "registration",
            
            ClientError::CallNotFound { .. } |
            ClientError::CallAlreadyExists { .. } |
            ClientError::InvalidCallState { .. } |
            ClientError::InvalidCallStateGeneric { .. } |
            ClientError::CallSetupFailed { .. } |
            ClientError::CallTerminated { .. } => "call",
            
            ClientError::MediaNegotiationFailed { .. } |
            ClientError::NoCompatibleCodecs |
            ClientError::AudioDeviceError { .. } |
            ClientError::UnsupportedCodec { .. } |
            ClientError::CodecError { .. } => "media",
            
            ClientError::NetworkError { .. } |
            ClientError::ConnectionTimeout |
            ClientError::ServerUnreachable { .. } |
            ClientError::TransportFailed { .. } |
            ClientError::TransportNotAvailable { .. } => "network",
            
            ClientError::ProtocolError { .. } |
            ClientError::InvalidSipMessage { .. } |
            ClientError::ProtocolVersionMismatch { .. } => "protocol",
            
            ClientError::InvalidConfiguration { .. } |
            ClientError::MissingConfiguration { .. } => "configuration",
            
            ClientError::SessionManagerError { .. } |
            ClientError::TooManySessions { .. } => "session",
            
            ClientError::InternalError { .. } |
            ClientError::OperationTimeout { .. } |
            ClientError::NotImplemented { .. } |
            ClientError::PermissionDenied { .. } |
            ClientError::ResourceUnavailable { .. } |
            ClientError::ExternalServiceError { .. } => "system",
        }
    }
} 