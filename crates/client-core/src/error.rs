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
//! ```rust
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
//! ```rust
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
//! ```rust
//! # use rvoip_client_core::{Client, ClientError};
//! # use std::sync::Arc;
//! # use std::time::Duration;
//! # async fn example(client: Arc<Client>) -> Result<(), Box<dyn std::error::Error>> {
//! match client.register_simple(
//!     "sip:alice@example.com",
//!     &"127.0.0.1:5060".parse().unwrap(),
//!     Duration::from_secs(3600)
//! ).await {
//!     Ok(()) => {
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
//! ```rust
//! # use rvoip_client_core::{Client, ClientError, CallId};
//! # use std::sync::Arc;
//! # async fn example(client: Arc<Client>, call_id: CallId) -> Result<(), Box<dyn std::error::Error>> {
//! // Safe call control with state checking
//! async fn safe_hold_call(client: &Arc<Client>, call_id: &CallId) -> Result<(), ClientError> {
//!     // Get call info first
//!     let info = client.get_call(call_id).await?;
//!     
//!     // Check if we can hold
//!     match info.state {
//!         rvoip_client_core::call::CallState::Connected => {
//!             client.hold_call(call_id).await
//!         }
//!         rvoip_client_core::call::CallState::Failed => {
//!             // Call failed, cannot hold
//!             Err(ClientError::InvalidCallStateGeneric {
//!                 expected: "Connected".to_string(),
//!                 actual: "Failed".to_string(),
//!             })
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
//! ```rust
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
//! ```rust
//! # use rvoip_client_core::{Client, ClientError, CallId};
//! # use std::sync::Arc;
//! # let call_id = CallId::new_v4();
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
//! ```rust
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
/// 
/// This enum covers all possible error conditions that can occur during
/// VoIP client operations, organized by functional area for easy handling.
/// 
/// # Error Categories
/// 
/// - **Registration Errors** - SIP server registration and authentication issues
/// - **Call Errors** - Call lifecycle and state management issues  
/// - **Media Errors** - Audio, codecs, and RTP stream issues
/// - **Network Errors** - Connectivity and transport problems
/// - **Protocol Errors** - SIP protocol violations and parsing issues
/// - **Configuration Errors** - Invalid settings and missing parameters
/// - **System Errors** - Internal failures and resource limitations
/// 
/// # Examples
/// 
/// ```rust
/// use rvoip_client_core::ClientError;
/// 
/// // Check error categories for appropriate handling
/// let error = ClientError::registration_failed("Invalid credentials");
/// assert_eq!(error.category(), "registration");
/// 
/// // Check authentication errors
/// let auth_error = ClientError::authentication_failed("Invalid password");
/// assert!(auth_error.is_auth_error());
/// 
/// // Create specific error types
/// let timeout_error = ClientError::OperationTimeout { duration_ms: 5000 };
/// assert!(timeout_error.is_recoverable());
/// ```
#[derive(Error, Debug, Clone)]
pub enum ClientError {
    /// Registration related errors
    
    /// Registration attempt failed due to server or authentication issues
    /// 
    /// This error occurs when the SIP REGISTER request is rejected by the server,
    /// typically due to authentication failures, server errors, or network issues.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientError;
    /// 
    /// let error = ClientError::RegistrationFailed {
    ///     reason: "401 Unauthorized - Invalid credentials".to_string()
    /// };
    /// 
    /// assert_eq!(error.category(), "registration");
    /// ```
    #[error("Registration failed: {reason}")]
    RegistrationFailed { 
        /// Detailed reason for the registration failure
        reason: String 
    },

    /// Client is not currently registered with any SIP server
    /// 
    /// This error occurs when attempting operations that require an active
    /// registration (like making calls) when no registration is active.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientError;
    /// 
    /// let error = ClientError::NotRegistered;
    /// assert!(error.is_auth_error());
    /// ```
    #[error("Not registered with server")]
    NotRegistered,

    /// SIP registration has expired and needs to be renewed
    /// 
    /// This error occurs when the registration lifetime has elapsed and
    /// the server no longer considers this client registered.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientError;
    /// 
    /// let error = ClientError::RegistrationExpired;
    /// assert!(error.is_auth_error());
    /// assert_eq!(error.category(), "registration");
    /// ```
    #[error("Registration expired")]
    RegistrationExpired,

    /// SIP digest authentication failed
    /// 
    /// This error occurs when the server rejects the authentication credentials
    /// provided during registration or call setup.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientError;
    /// 
    /// let error = ClientError::AuthenticationFailed {
    ///     reason: "Wrong password for user alice".to_string()
    /// };
    /// 
    /// assert!(error.is_auth_error());
    /// ```
    #[error("Authentication failed: {reason}")]
    AuthenticationFailed { 
        /// Specific reason for authentication failure
        reason: String 
    },

    /// Call related errors

    /// Attempted to operate on a call that doesn't exist
    /// 
    /// This error occurs when referencing a call ID that is not found
    /// in the client's active call list.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientError;
    /// use uuid::Uuid;
    /// 
    /// let call_id = Uuid::new_v4();
    /// let error = ClientError::CallNotFound { call_id };
    /// 
    /// assert!(error.is_call_error());
    /// assert_eq!(error.category(), "call");
    /// ```
    #[error("Call not found: {call_id}")]
    CallNotFound { 
        /// The call ID that was not found
        call_id: Uuid 
    },

    /// Attempted to create a call with an ID that already exists
    /// 
    /// This error occurs when trying to create a new call with a call ID
    /// that is already in use by another active call.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientError;
    /// use uuid::Uuid;
    /// 
    /// let call_id = Uuid::new_v4();
    /// let error = ClientError::CallAlreadyExists { call_id };
    /// 
    /// assert!(error.is_call_error());
    /// ```
    #[error("Call already exists: {call_id}")]
    CallAlreadyExists { 
        /// The call ID that already exists
        call_id: Uuid 
    },

    /// Operation is not valid for the current call state
    /// 
    /// This error occurs when attempting an operation that is not allowed
    /// in the call's current state (e.g., trying to answer an already connected call).
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientError;
    /// use rvoip_client_core::call::CallState;
    /// use uuid::Uuid;
    /// 
    /// let call_id = Uuid::new_v4();
    /// let error = ClientError::InvalidCallState {
    ///     call_id,
    ///     current_state: CallState::Terminated
    /// };
    /// 
    /// assert!(error.is_call_error());
    /// ```
    #[error("Invalid call state for call {call_id}: current state is {current_state:?}")]
    InvalidCallState { 
        /// The call ID that has an invalid state
        call_id: Uuid, 
        /// The current state that prevents the operation
        current_state: crate::call::CallState 
    },

    /// Generic call state validation error with string descriptions
    /// 
    /// This error provides a more flexible way to report call state issues
    /// when the specific CallState enum values are not available.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientError;
    /// 
    /// let error = ClientError::InvalidCallStateGeneric {
    ///     expected: "Connected".to_string(),
    ///     actual: "Terminated".to_string()
    /// };
    /// 
    /// assert!(error.is_call_error());
    /// ```
    #[error("Invalid call state: expected {expected}, got {actual}")]
    InvalidCallStateGeneric { 
        /// The expected call state for the operation
        expected: String, 
        /// The actual call state that was encountered
        actual: String 
    },

    /// Call establishment or setup failed
    /// 
    /// This error occurs during the call setup phase when the INVITE
    /// request fails due to network, server, or remote party issues.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientError;
    /// 
    /// let error = ClientError::CallSetupFailed {
    ///     reason: "486 Busy Here".to_string()
    /// };
    /// 
    /// assert!(error.is_call_error());
    /// ```
    #[error("Call setup failed: {reason}")]
    CallSetupFailed { 
        /// Specific reason for call setup failure
        reason: String 
    },

    /// Call was terminated unexpectedly
    /// 
    /// This error occurs when a call ends due to network issues,
    /// remote party disconnect, or other unexpected conditions.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientError;
    /// 
    /// let error = ClientError::CallTerminated {
    ///     reason: "Network connection lost".to_string()
    /// };
    /// 
    /// assert!(error.is_call_error());
    /// ```
    #[error("Call terminated: {reason}")]
    CallTerminated { 
        /// Reason for call termination
        reason: String 
    },

    /// Media related errors

    /// SDP negotiation or media setup failed
    /// 
    /// This error occurs when the client cannot establish media streams
    /// due to codec mismatches, network issues, or SDP parsing problems.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientError;
    /// 
    /// let error = ClientError::MediaNegotiationFailed {
    ///     reason: "No common codecs found".to_string()
    /// };
    /// 
    /// assert_eq!(error.category(), "media");
    /// ```
    #[error("Media negotiation failed: {reason}")]
    MediaNegotiationFailed { 
        /// Specific reason for media negotiation failure
        reason: String 
    },
    
    /// General media processing error
    /// 
    /// This error covers various media-related issues including RTP
    /// processing problems, audio device failures, and codec errors.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientError;
    /// 
    /// let error = ClientError::MediaError {
    ///     details: "RTP packet processing failed".to_string()
    /// };
    /// 
    /// assert_eq!(error.category(), "media");
    /// ```
    #[error("Media error: {details}")]
    MediaError { 
        /// Detailed description of the media error
        details: String 
    },

    /// No audio codecs are compatible between endpoints
    /// 
    /// This error occurs when the local and remote endpoints cannot
    /// agree on a common audio codec for the call.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientError;
    /// 
    /// let error = ClientError::NoCompatibleCodecs;
    /// assert_eq!(error.category(), "media");
    /// ```
    #[error("No compatible codecs")]
    NoCompatibleCodecs,

    /// Audio device (microphone/speaker) error
    /// 
    /// This error occurs when there are problems accessing or using
    /// the system's audio input/output devices.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientError;
    /// 
    /// let error = ClientError::AudioDeviceError {
    ///     reason: "Microphone permission denied".to_string()
    /// };
    /// 
    /// assert_eq!(error.category(), "media");
    /// ```
    #[error("Audio device error: {reason}")]
    AudioDeviceError { 
        /// Specific reason for audio device failure
        reason: String 
    },

    /// Network and transport errors

    /// General network connectivity or communication error
    /// 
    /// This error occurs for various network-related issues including
    /// DNS resolution failures, socket errors, and packet loss.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientError;
    /// 
    /// let error = ClientError::NetworkError {
    ///     reason: "DNS resolution failed for server.example.com".to_string()
    /// };
    /// 
    /// assert!(error.is_recoverable());
    /// assert_eq!(error.category(), "network");
    /// ```
    #[error("Network error: {reason}")]
    NetworkError { 
        /// Specific description of the network error
        reason: String 
    },

    /// Operation timed out waiting for network response
    /// 
    /// This error occurs when network operations (like SIP requests)
    /// do not receive a response within the configured timeout period.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientError;
    /// 
    /// let error = ClientError::ConnectionTimeout;
    /// assert!(error.is_recoverable());
    /// assert_eq!(error.category(), "network");
    /// ```
    #[error("Connection timeout")]
    ConnectionTimeout,

    /// Cannot reach the specified SIP server
    /// 
    /// This error occurs when the client cannot establish a connection
    /// to the SIP server due to network issues or incorrect server address.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientError;
    /// 
    /// let error = ClientError::ServerUnreachable {
    ///     server: "sip.example.com:5060".to_string()
    /// };
    /// 
    /// assert_eq!(error.category(), "network");
    /// ```
    #[error("Server unreachable: {server}")]
    ServerUnreachable { 
        /// The server address that could not be reached
        server: String 
    },

    /// Protocol errors

    /// SIP protocol violation or parsing error
    /// 
    /// This error occurs when receiving malformed SIP messages or
    /// encountering protocol violations that prevent proper communication.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientError;
    /// 
    /// let error = ClientError::ProtocolError {
    ///     reason: "Invalid SIP URI format".to_string()
    /// };
    /// 
    /// assert_eq!(error.category(), "protocol");
    /// ```
    #[error("SIP protocol error: {reason}")]
    ProtocolError { 
        /// Specific description of the protocol error
        reason: String 
    },

    /// Received an invalid or malformed SIP message
    /// 
    /// This error occurs when parsing SIP messages that do not conform
    /// to the SIP specification or contain invalid syntax.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientError;
    /// 
    /// let error = ClientError::InvalidSipMessage {
    ///     reason: "Missing required Via header".to_string()
    /// };
    /// 
    /// assert_eq!(error.category(), "protocol");
    /// ```
    #[error("Invalid SIP message: {reason}")]
    InvalidSipMessage { 
        /// Specific reason why the SIP message is invalid
        reason: String 
    },

    /// SIP protocol version mismatch
    /// 
    /// This error occurs when the client and server are using incompatible
    /// versions of the SIP protocol.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientError;
    /// 
    /// let error = ClientError::ProtocolVersionMismatch {
    ///     expected: "SIP/2.0".to_string(),
    ///     actual: "SIP/3.0".to_string()
    /// };
    /// 
    /// assert_eq!(error.category(), "protocol");
    /// ```
    #[error("Protocol version mismatch: expected {expected}, got {actual}")]
    ProtocolVersionMismatch { 
        /// The expected protocol version
        expected: String, 
        /// The actual protocol version received
        actual: String 
    },

    /// Configuration errors

    /// Client configuration is invalid
    /// 
    /// This error occurs when the client is configured with invalid
    /// settings that prevent proper operation.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientError;
    /// 
    /// let error = ClientError::InvalidConfiguration {
    ///     field: "sip_port".to_string(),
    ///     reason: "Port must be between 1024 and 65535".to_string()
    /// };
    /// 
    /// assert_eq!(error.category(), "configuration");
    /// ```
    #[error("Invalid configuration: {field} - {reason}")]
    InvalidConfiguration { 
        /// The configuration field that is invalid
        field: String, 
        /// Explanation of why the configuration is invalid
        reason: String 
    },

    /// Required configuration parameter is missing
    /// 
    /// This error occurs when mandatory configuration values are not
    /// provided to the client.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientError;
    /// 
    /// let error = ClientError::MissingConfiguration {
    ///     field: "server_address".to_string()
    /// };
    /// 
    /// assert_eq!(error.category(), "configuration");
    /// ```
    #[error("Missing required configuration: {field}")]
    MissingConfiguration { 
        /// The configuration field that is missing
        field: String 
    },

    /// Transport errors

    /// Network transport layer failure
    /// 
    /// This error occurs when the underlying transport (UDP/TCP/TLS)
    /// encounters failures that prevent SIP communication.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientError;
    /// 
    /// let error = ClientError::TransportFailed {
    ///     reason: "TLS handshake failed".to_string()
    /// };
    /// 
    /// assert!(error.is_recoverable());
    /// assert_eq!(error.category(), "network");
    /// ```
    #[error("Transport failed: {reason}")]
    TransportFailed { 
        /// Specific reason for transport failure
        reason: String 
    },

    /// Requested transport type is not available
    /// 
    /// This error occurs when trying to use a transport protocol
    /// that is not supported or configured in the client.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientError;
    /// 
    /// let error = ClientError::TransportNotAvailable {
    ///     transport_type: "WSS".to_string()
    /// };
    /// 
    /// assert_eq!(error.category(), "network");
    /// ```
    #[error("Transport not available: {transport_type}")]
    TransportNotAvailable { 
        /// The transport type that is not available
        transport_type: String 
    },

    /// Session management errors

    /// Session manager internal error
    /// 
    /// This error occurs when the session management layer encounters
    /// internal failures that prevent proper session handling.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientError;
    /// 
    /// let error = ClientError::SessionManagerError {
    ///     reason: "Session state corruption detected".to_string()
    /// };
    /// 
    /// assert_eq!(error.category(), "session");
    /// ```
    #[error("Session manager error: {reason}")]
    SessionManagerError { 
        /// Specific reason for session manager failure
        reason: String 
    },

    /// Maximum number of concurrent sessions exceeded
    /// 
    /// This error occurs when attempting to create more sessions than
    /// the configured or system-imposed limit allows.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientError;
    /// 
    /// let error = ClientError::TooManySessions {
    ///     limit: 10
    /// };
    /// 
    /// assert_eq!(error.category(), "session");
    /// ```
    #[error("Too many sessions: limit is {limit}")]
    TooManySessions { 
        /// The maximum number of sessions allowed
        limit: usize 
    },

    /// Generic errors

    /// Internal client error
    /// 
    /// This error indicates an unexpected internal failure within the
    /// client library that should not normally occur.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientError;
    /// 
    /// let error = ClientError::InternalError {
    ///     message: "Unexpected null pointer in call handler".to_string()
    /// };
    /// 
    /// assert_eq!(error.category(), "system");
    /// ```
    #[error("Internal error: {message}")]
    InternalError { 
        /// Description of the internal error
        message: String 
    },

    /// Operation exceeded its timeout deadline
    /// 
    /// This error occurs when an operation takes longer than its
    /// configured timeout period to complete.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientError;
    /// 
    /// let error = ClientError::OperationTimeout {
    ///     duration_ms: 5000
    /// };
    /// 
    /// assert_eq!(error.category(), "system");
    /// ```
    #[error("Operation timeout after {duration_ms}ms")]
    OperationTimeout { 
        /// The timeout duration in milliseconds
        duration_ms: u64 
    },

    /// Requested feature is not yet implemented
    /// 
    /// This error occurs when attempting to use functionality that
    /// is planned but not yet available in the current version.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientError;
    /// 
    /// let error = ClientError::NotImplemented {
    ///     feature: "Video calls".to_string(),
    ///     reason: "Planned for version 2.0".to_string()
    /// };
    /// 
    /// assert_eq!(error.category(), "system");
    /// ```
    #[error("Not implemented: {feature} - {reason}")]
    NotImplemented { 
        /// The feature that is not implemented
        feature: String, 
        /// Explanation of implementation status
        reason: String 
    },

    /// Operation not permitted in current context
    /// 
    /// This error occurs when the user or system lacks the necessary
    /// permissions to perform the requested operation.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientError;
    /// 
    /// let error = ClientError::PermissionDenied {
    ///     operation: "Access microphone".to_string()
    /// };
    /// 
    /// assert_eq!(error.category(), "system");
    /// ```
    #[error("Permission denied: {operation}")]
    PermissionDenied { 
        /// The operation that was denied
        operation: String 
    },

    /// Required resource is not available
    /// 
    /// This error occurs when the system cannot allocate or access
    /// a required resource (memory, ports, devices, etc.).
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientError;
    /// 
    /// let error = ClientError::ResourceUnavailable {
    ///     resource: "RTP port range 10000-20000".to_string()
    /// };
    /// 
    /// assert_eq!(error.category(), "system");
    /// ```
    #[error("Resource unavailable: {resource}")]
    ResourceUnavailable { 
        /// The resource that is unavailable
        resource: String 
    },

    /// Codec and media format errors

    /// Audio codec is not supported
    /// 
    /// This error occurs when attempting to use an audio codec that
    /// is not available in the current client configuration.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientError;
    /// 
    /// let error = ClientError::UnsupportedCodec {
    ///     codec: "G.729".to_string()
    /// };
    /// 
    /// assert_eq!(error.category(), "media");
    /// ```
    #[error("Unsupported codec: {codec}")]
    UnsupportedCodec { 
        /// The codec that is not supported
        codec: String 
    },

    /// Codec processing error
    /// 
    /// This error occurs when there are problems encoding or decoding
    /// audio using the selected codec.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientError;
    /// 
    /// let error = ClientError::CodecError {
    ///     reason: "OPUS encoder initialization failed".to_string()
    /// };
    /// 
    /// assert_eq!(error.category(), "media");
    /// ```
    #[error("Codec error: {reason}")]
    CodecError { 
        /// Specific reason for codec error
        reason: String 
    },

    /// External service errors

    /// External service or dependency failure
    /// 
    /// This error occurs when an external service (STUN server, media relay,
    /// authentication service, etc.) fails or becomes unavailable.
    /// 
    /// # Examples
    /// 
    /// ```rust
    /// use rvoip_client_core::ClientError;
    /// 
    /// let error = ClientError::ExternalServiceError {
    ///     service: "STUN server".to_string(),
    ///     reason: "stun.example.com connection refused".to_string()
    /// };
    /// 
    /// assert_eq!(error.category(), "system");
    /// ```
    #[error("External service error: {service} - {reason}")]
    ExternalServiceError { 
        /// The external service that failed
        service: String, 
        /// Specific reason for the service failure
        reason: String 
    },
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