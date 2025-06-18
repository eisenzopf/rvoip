//! Error types and handling for the client-core library
//! 
//! This module defines all error types that can occur during client operations
//! and provides guidance on how to handle them.
//! 
//! # Error Categories
//! 
//! Errors are categorized to help with recovery strategies:
//! 
//! - **Configuration Errors** - Invalid settings, can't recover without fixing config
//! - **Network Errors** - Temporary network issues, usually recoverable with retry
//! - **Protocol Errors** - SIP protocol violations, may need different approach
//! - **Media Errors** - Audio/RTP issues, might need codec renegotiation
//! - **State Errors** - Invalid operation for current state, check state first
//! 
//! # Error Handling Guide
//! 
//! ## Basic Pattern
//! 
//! ```rust,no_run
//! # use rvoip_client_core::{Client, ClientError};
//! # use std::sync::Arc;
//! # async fn example(client: Arc<Client>) {
//! match client.make_call(
//!     "sip:alice@example.com".to_string(),
//!     "sip:bob@example.com".to_string(),
//!     None
//! ).await {
//!     Ok(call_id) => {
//!         println!("Call started: {}", call_id);
//!     }
//!     Err(ClientError::NetworkError { reason }) => {
//!         eprintln!("Network problem: {}", reason);
//!         // Retry after checking network connectivity
//!     }
//!     Err(ClientError::InvalidConfiguration { field, reason }) => {
//!         eprintln!("Config error in {}: {}", field, reason);
//!         // Fix configuration before retrying
//!     }
//!     Err(e) => {
//!         eprintln!("Unexpected error: {}", e);
//!         // Log and notify user
//!     }
//! }
//! # }
//! ```
//! 
//! ## Recovery Strategies
//! 
//! ### Network Errors
//! 
//! Network errors are often temporary. Implement exponential backoff:
//! 
//! ```rust,no_run
//! # use rvoip_client_core::{Client, ClientError};
//! # use std::sync::Arc;
//! # use std::time::Duration;
//! # async fn example(client: Arc<Client>) -> Result<(), Box<dyn std::error::Error>> {
//! async fn with_retry<T, F, Fut>(
//!     mut operation: F,
//!     max_attempts: u32,
//! ) -> Result<T, ClientError>
//! where
//!     F: FnMut() -> Fut,
//!     Fut: std::future::Future<Output = Result<T, ClientError>>,
//! {
//!     let mut attempt = 0;
//!     let mut delay = Duration::from_millis(100);
//!     
//!     loop {
//!         match operation().await {
//!             Ok(result) => return Ok(result),
//!             Err(e) if e.is_recoverable() && attempt < max_attempts => {
//!                 attempt += 1;
//!                 tokio::time::sleep(delay).await;
//!                 delay *= 2; // Exponential backoff
//!             }
//!             Err(e) => return Err(e),
//!         }
//!     }
//! }
//! 
//! // Use with any operation
//! let call_id = with_retry(|| async {
//!     client.make_call(
//!         "sip:alice@example.com".to_string(),
//!         "sip:bob@example.com".to_string(),
//!         None
//!     ).await
//! }, 3).await?;
//! # Ok(())
//! # }
//! ```
//! 
//! ### Registration Errors
//! 
//! Handle authentication and server errors:
//! 
//! ```rust,no_run
//! # use rvoip_client_core::{Client, ClientError};
//! # use std::sync::Arc;
//! # use std::time::Duration;
//! # async fn example(client: Arc<Client>) -> Result<(), Box<dyn std::error::Error>> {
//! match client.register_simple(
//!     "sip:alice@example.com",
//!     "registrar.example.com:5060",
//!     Duration::from_secs(3600)
//! ).await {
//!     Ok(reg_id) => {
//!         println!("Registered successfully");
//!     }
//!     Err(e) if e.is_auth_error() => {
//!         // Prompt user for credentials
//!         println!("Please check your username and password");
//!     }
//!     Err(ClientError::RegistrationFailed { reason }) => {
//!         if reason.contains("timeout") {
//!             // Server might be down
//!             println!("Server not responding, try again later");
//!         } else if reason.contains("forbidden") {
//!             // Account might be disabled
//!             println!("Registration forbidden, contact support");
//!         }
//!     }
//!     Err(e) => eprintln!("Registration error: {}", e),
//! }
//! # Ok(())
//! # }
//! ```
//! 
//! ### Call State Errors
//! 
//! Check state before operations:
//! 
//! ```rust,no_run
//! # use rvoip_client_core::{Client, ClientError, CallId};
//! # use std::sync::Arc;
//! # async fn example(client: Arc<Client>, call_id: CallId) -> Result<(), Box<dyn std::error::Error>> {
//! // Safe call control with state checking
//! async fn safe_hold_call(client: &Arc<Client>, call_id: &CallId) -> Result<(), ClientError> {
//!     // Get call info first
//!     let info = client.get_call_info(call_id).await?
//!         .ok_or(ClientError::CallNotFound { call_id: *call_id })?;
//!     
//!     // Check if we can hold
//!     match info.state {
//!         rvoip_client_core::call::CallState::Connected => {
//!             client.hold_call(call_id).await
//!         }
//!         rvoip_client_core::call::CallState::OnHold => {
//!             // Already on hold
//!             Ok(())
//!         }
//!         _ => {
//!             Err(ClientError::InvalidCallStateGeneric {
//!                 expected: "Connected".to_string(),
//!                 actual: format!("{:?}", info.state),
//!             })
//!         }
//!     }
//! }
//! 
//! safe_hold_call(&client, &call_id).await?;
//! # Ok(())
//! # }
//! ```
//! 
//! ### Media Errors
//! 
//! Handle codec and port allocation issues:
//! 
//! ```rust,no_run
//! # use rvoip_client_core::{Client, ClientError, CallId};
//! # use std::sync::Arc;
//! # async fn example(client: Arc<Client>, call_id: CallId) -> Result<(), Box<dyn std::error::Error>> {
//! match client.establish_media(&call_id, "remote.example.com:30000").await {
//!     Ok(_) => println!("Media established"),
//!     Err(ClientError::MediaNegotiationFailed { reason }) => {
//!         if reason.contains("codec") {
//!             // Try with different codec
//!             println!("Codec mismatch, trying fallback");
//!         } else if reason.contains("port") {
//!             // Port allocation failed
//!             println!("No available media ports");
//!         }
//!     }
//!     Err(e) => eprintln!("Media setup failed: {}", e),
//! }
//! # Ok(())
//! # }
//! ```
//! 
//! ## Error Context
//! 
//! Always log errors with context for debugging:
//! 
//! ```rust,no_run
//! # use rvoip_client_core::{Client, ClientError, CallId};
//! # use std::sync::Arc;
//! # let call_id = CallId::new();
//! # async fn example(client: Arc<Client>, call_id: CallId) {
//! use tracing::{error, warn, info};
//! 
//! match client.answer_call(&call_id).await {
//!     Ok(_) => info!(call_id = %call_id, "Call answered successfully"),
//!     Err(e) => {
//!         error!(
//!             call_id = %call_id,
//!             error = %e,
//!             error_type = ?e,
//!             category = e.category(),
//!             "Failed to answer call"
//!         );
//!         
//!         // Take appropriate action based on error type
//!         match e {
//!             ClientError::CallNotFound { .. } => {
//!                 // Call might have been cancelled
//!             }
//!             ClientError::MediaNegotiationFailed { .. } => {
//!                 // Try to recover media session
//!             }
//!             _ => {
//!                 // Generic error handling
//!             }
//!         }
//!     }
//! }
//! # }
//! ```
//! 
//! ## Error Categories Helper
//! 
//! Use the `category()` method to group errors for metrics:
//! 
//! ```rust,no_run
//! # use rvoip_client_core::ClientError;
//! # use std::collections::HashMap;
//! # let errors: Vec<ClientError> = vec![];
//! let mut error_counts: HashMap<&'static str, usize> = HashMap::new();
//! 
//! for error in errors {
//!     *error_counts.entry(error.category()).or_insert(0) += 1;
//! }
//! 
//! // Report metrics
//! for (category, count) in error_counts {
//!     println!("{}: {} errors", category, count);
//! }
//! ```

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
    
    #[error("Media error: {details}")]
    MediaError { details: String },

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
                | ClientError::InvalidCallStateGeneric { .. }
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
            ClientError::MediaError { .. } |
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