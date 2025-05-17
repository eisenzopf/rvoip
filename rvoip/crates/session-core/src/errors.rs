use std::fmt;
use thiserror::Error;

/// Error category for grouping and classification
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorCategory {
    /// Errors related to network issues
    Network,
    
    /// Errors related to SIP protocol processing
    Protocol,
    
    /// Errors related to session management
    Session,
    
    /// Errors related to dialog handling
    Dialog,
    
    /// Errors related to media handling
    Media,
    
    /// Errors related to authentication
    Authentication,
    
    /// Errors related to resource limitations
    Resource,
    
    /// Errors related to configuration issues
    Configuration,
    
    /// Errors related to user input validation
    Validation,
    
    /// Errors related to timeout conditions
    Timeout,
    
    /// Errors from external dependencies
    External,
    
    /// Unexpected internal errors
    Internal,
}

impl fmt::Display for ErrorCategory {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErrorCategory::Network => write!(f, "Network"),
            ErrorCategory::Protocol => write!(f, "Protocol"),
            ErrorCategory::Session => write!(f, "Session"),
            ErrorCategory::Dialog => write!(f, "Dialog"),
            ErrorCategory::Media => write!(f, "Media"),
            ErrorCategory::Authentication => write!(f, "Authentication"),
            ErrorCategory::Resource => write!(f, "Resource"),
            ErrorCategory::Configuration => write!(f, "Configuration"),
            ErrorCategory::Validation => write!(f, "Validation"),
            ErrorCategory::Timeout => write!(f, "Timeout"),
            ErrorCategory::External => write!(f, "External"),
            ErrorCategory::Internal => write!(f, "Internal"),
        }
    }
}

/// Error severity for operational response
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ErrorSeverity {
    /// Informational issue, operation can continue
    Info,
    
    /// Warning that may affect quality but not functionality
    Warning,
    
    /// Error that prevents an operation but system can continue
    Error,
    
    /// Critical error that may require system restart
    Critical,
}

impl fmt::Display for ErrorSeverity {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            ErrorSeverity::Info => write!(f, "Info"),
            ErrorSeverity::Warning => write!(f, "Warning"),
            ErrorSeverity::Error => write!(f, "Error"),
            ErrorSeverity::Critical => write!(f, "Critical"),
        }
    }
}

/// Recovery action suggestion
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum RecoveryAction {
    /// No action needed, informational only
    None,
    
    /// Retry the operation
    Retry,
    
    /// Retry with backoff
    RetryWithBackoff(std::time::Duration),
    
    /// Reconnect to the service
    Reconnect,
    
    /// Recreate the session
    RecreateSession,
    
    /// Check configuration parameters
    CheckConfiguration(String),
    
    /// Wait for system recovery
    Wait(std::time::Duration),
    
    /// Restart the component
    RestartComponent(String),
    
    /// Custom action description
    Custom(String),
}

impl fmt::Display for RecoveryAction {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            RecoveryAction::None => write!(f, "No action required"),
            RecoveryAction::Retry => write!(f, "Retry the operation"),
            RecoveryAction::RetryWithBackoff(duration) => 
                write!(f, "Retry with backoff period of {:?}", duration),
            RecoveryAction::Reconnect => write!(f, "Reconnect to the service"),
            RecoveryAction::RecreateSession => write!(f, "Recreate the session"),
            RecoveryAction::CheckConfiguration(param) => 
                write!(f, "Check configuration parameter: {}", param),
            RecoveryAction::Wait(duration) => 
                write!(f, "Wait for {:?} before retrying", duration),
            RecoveryAction::RestartComponent(component) => 
                write!(f, "Restart component: {}", component),
            RecoveryAction::Custom(action) => write!(f, "{}", action),
        }
    }
}

/// Enhanced context for errors with detailed metadata
#[derive(Debug, Clone)]
pub struct ErrorContext {
    /// Category of the error
    pub category: ErrorCategory,
    
    /// Severity of the error
    pub severity: ErrorSeverity,
    
    /// Suggested recovery action
    pub recovery: RecoveryAction,
    
    /// Whether the error is retryable
    pub retryable: bool,
    
    /// Related transaction ID if applicable
    pub transaction_id: Option<String>,
    
    /// Related session ID if applicable
    pub session_id: Option<String>,
    
    /// Related dialog ID if applicable
    pub dialog_id: Option<String>,
    
    /// Error timestamp
    pub timestamp: std::time::SystemTime,
    
    /// Additional context information
    pub details: Option<String>,
}

impl Default for ErrorContext {
    fn default() -> Self {
        Self {
            category: ErrorCategory::Internal,
            severity: ErrorSeverity::Error,
            recovery: RecoveryAction::None,
            retryable: false,
            transaction_id: None,
            session_id: None,
            dialog_id: None,
            timestamp: std::time::SystemTime::now(),
            details: None,
        }
    }
}

impl std::error::Error for ErrorContext {}

impl std::fmt::Display for ErrorContext {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "Error context [{}:{}]", self.category, self.severity)?;
        if let Some(details) = &self.details {
            write!(f, ": {}", details)?;
        }
        Ok(())
    }
}

impl ErrorContext {
    pub fn with_message(mut self, message: &str) -> Self {
        self.details = Some(message.to_string());
        self
    }
}

/// Errors related to session management with enhanced context
#[derive(Error, Debug)]
pub enum Error {
    //
    // Session-related errors
    //
    
    /// Session not found with ID
    #[error("Session not found with ID: {0}")]
    SessionNotFoundWithId(String, ErrorContext),

    /// Session already exists
    #[error("Session already exists: {0}")]
    SessionAlreadyExists(String, ErrorContext),

    /// Session already terminated
    #[error("Session already terminated: {0}")]
    SessionTerminated(String, ErrorContext),

    /// Invalid session state transition
    #[error("Invalid session state transition from {from} to {to}")]
    InvalidSessionStateTransition {
        from: String,
        to: String,
        context: ErrorContext,
    },

    /// Session limit exceeded
    #[error("Session limit exceeded (max: {0})")]
    SessionLimitExceeded(usize, ErrorContext),

    //
    // Dialog-related errors
    //
    
    /// Dialog not found with ID
    #[error("Dialog not found with ID: {0}")]
    DialogNotFoundWithId(String, ErrorContext),

    /// Dialog not found for request
    #[error("Dialog not found for request")]
    DialogNotFound(ErrorContext),

    /// No active dialog in session
    #[error("No active dialog in session: {0}")]
    NoActiveDialog(String, ErrorContext),

    /// Dialog already exists
    #[error("Dialog already exists: {0}")]
    DialogAlreadyExists(String, ErrorContext),

    /// Invalid dialog state
    #[error("Invalid dialog state: current={current}, expected={expected}")]
    InvalidDialogState {
        current: String,
        expected: String,
        context: ErrorContext,
    },

    /// Dialog creation failed
    #[error("Dialog creation failed: {0}")]
    DialogCreationFailed(String, ErrorContext),

    /// Dialog update failed
    #[error("Dialog update failed: {0}")]
    DialogUpdateFailed(String, ErrorContext),

    //
    // Transaction-related errors
    //
    
    /// Transaction not found
    #[error("Transaction not found: {0}")]
    TransactionNotFound(String, ErrorContext),

    /// Transaction failed
    #[error("Transaction failed: {0}")]
    TransactionFailed(String, Option<Box<dyn std::error::Error + Send + Sync>>, ErrorContext),

    /// Transaction not associated with session
    #[error("Transaction not associated with session: {0}")]
    TransactionNotAssociated(String, ErrorContext),

    /// Transaction timeout
    #[error("Transaction timeout: {0}")]
    TransactionTimeout(String, ErrorContext),

    /// Transaction creation failed
    #[error("Transaction creation failed: {0}")]
    TransactionCreationFailed(String, Option<Box<dyn std::error::Error + Send + Sync>>, ErrorContext),

    //
    // Network-related errors
    //
    
    /// Cannot resolve destination
    #[error("Cannot resolve request destination: {0}")]
    CannotResolveDestination(String, ErrorContext),

    /// Network unreachable
    #[error("Network unreachable: {0}")]
    NetworkUnreachable(String, ErrorContext),

    /// Connection failed
    #[error("Connection failed: {0}")]
    ConnectionFailed(String, ErrorContext),

    /// Connection closed
    #[error("Connection closed: {0}")]
    ConnectionClosed(String, ErrorContext),

    //
    // Media-related errors
    //
    
    /// Media negotiation error
    #[error("Media negotiation error: {0}")]
    MediaNegotiationError(String, ErrorContext),

    /// Media stream error
    #[error("Media stream error: {0}")]
    MediaStreamError(String, ErrorContext),

    /// Codec incompatible
    #[error("Codec incompatible: {0}")]
    CodecIncompatible(String, ErrorContext),

    /// Media resource allocation error
    #[error("Media resource allocation error: {0}")]
    MediaResourceError(String, ErrorContext),

    /// SDP processing error
    #[error("SDP processing error: {0}")]
    SdpError(String, ErrorContext),

    //
    // Authentication-related errors
    //
    
    /// Authentication failed
    #[error("Authentication failed: {0}")]
    AuthenticationFailed(String, ErrorContext),

    /// Authentication challenge received, credentials required
    #[error("Authentication challenge: {challenge}")]
    AuthChallenge {
        /// The challenge details
        challenge: String,
        /// Error context
        context: ErrorContext,
    },

    /// Invalid credentials
    #[error("Invalid credentials: {0}")]
    InvalidCredentials(String, ErrorContext),

    //
    // Protocol-related errors
    //
    
    /// Invalid request
    #[error("Invalid request: {0}")]
    InvalidRequest(String, ErrorContext),

    /// Invalid response
    #[error("Invalid response: {0}")]
    InvalidResponse(String, ErrorContext),

    /// Invalid header
    #[error("Invalid header: {0}")]
    InvalidHeader(String, ErrorContext),

    /// Missing required header
    #[error("Missing required header: {0}")]
    MissingHeader(String, ErrorContext),

    /// Unsupported method
    #[error("Unsupported method: {0}")]
    UnsupportedMethod(String, ErrorContext),

    //
    // Timeout errors
    //
    
    /// Operation timeout
    #[error("Operation timeout: {0}")]
    OperationTimeout(String, ErrorContext),

    /// Response timeout
    #[error("Response timeout: {0}")]
    ResponseTimeout(String, ErrorContext),

    /// Dialog timeout
    #[error("Dialog timeout: {0}")]
    DialogTimeout(String, ErrorContext),

    //
    // Resource errors
    //
    
    /// Resource allocation failed
    #[error("Resource allocation failed: {0}")]
    ResourceAllocationFailed(String, ErrorContext),

    /// Resource limit exceeded
    #[error("Resource limit exceeded: {0}")]
    ResourceLimitExceeded(String, ErrorContext),

    /// Memory allocation failed
    #[error("Memory allocation failed: {0}")]
    MemoryAllocationFailed(String, ErrorContext),

    //
    // External dependency errors
    //
    
    /// Transaction layer error
    #[error("Transaction layer error: {0}")]
    TransactionError(
        rvoip_transaction_core::Error, 
        ErrorContext
    ),

    /// SIP protocol error
    #[error("SIP protocol error: {0}")]
    SipError(
        rvoip_sip_core::Error, 
        ErrorContext
    ),

    /// RTP error
    #[error("RTP error: {0}")]
    RtpError(
        rvoip_rtp_core::Error, 
        ErrorContext
    ),

    /// Media processing error
    #[error("Media processing error: {0}")]
    MediaError(
        rvoip_media_core::Error, 
        ErrorContext
    ),

    /// I/O error
    #[error("I/O error: {0}")]
    IoError(
        std::io::Error, 
        ErrorContext
    ),

    //
    // Other errors
    //
    
    /// Configuration error
    #[error("Configuration error: {0}")]
    ConfigurationError(String, ErrorContext),

    /// Internal error
    #[error("Internal error: {0}")]
    InternalError(String, ErrorContext),

    /// Unexpected error
    #[error("Unexpected error: {0}")]
    UnexpectedError(String, Option<Box<dyn std::error::Error + Send + Sync>>, ErrorContext),
    
    /// Serialization error
    #[error("Serialization error: {0}")]
    SerializationError(String, ErrorContext),

    /// Invalid media state
    #[error("Invalid media state")]
    InvalidMediaState {
        /// Error context
        context: ErrorContext,
    },

    /// Transport error
    #[error("Transport error: {0}")]
    TransportError(rvoip_sip_transport::error::Error, ErrorContext),

    /// Feature not supported
    #[error("Feature not supported: {feature}")]
    Unsupported {
        /// The feature that is not supported
        feature: String,
        /// Error context
        context: ErrorContext,
    },

    /// Missing required dialog data
    #[error("Missing required dialog data")]
    MissingDialogData {
        /// Error context 
        context: ErrorContext,
    },
}

impl Error {
    /// Get the error context
    pub fn context(&self) -> &ErrorContext {
        match self {
            Error::SessionNotFoundWithId(_, ctx) => ctx,
            Error::SessionAlreadyExists(_, ctx) => ctx,
            Error::SessionTerminated(_, ctx) => ctx,
            Error::InvalidSessionStateTransition { context, .. } => context,
            Error::SessionLimitExceeded(_, ctx) => ctx,
            Error::DialogNotFoundWithId(_, ctx) => ctx,
            Error::DialogNotFound(ctx) => ctx,
            Error::NoActiveDialog(_, ctx) => ctx,
            Error::DialogAlreadyExists(_, ctx) => ctx,
            Error::InvalidDialogState { context, .. } => context,
            Error::DialogCreationFailed(_, ctx) => ctx,
            Error::DialogUpdateFailed(_, ctx) => ctx,
            Error::TransactionNotFound(_, ctx) => ctx,
            Error::TransactionFailed(_, _, ctx) => ctx,
            Error::TransactionNotAssociated(_, ctx) => ctx,
            Error::TransactionTimeout(_, ctx) => ctx,
            Error::TransactionCreationFailed(_, _, ctx) => ctx,
            Error::CannotResolveDestination(_, ctx) => ctx,
            Error::NetworkUnreachable(_, ctx) => ctx,
            Error::ConnectionFailed(_, ctx) => ctx,
            Error::ConnectionClosed(_, ctx) => ctx,
            Error::MediaNegotiationError(_, ctx) => ctx,
            Error::MediaStreamError(_, ctx) => ctx,
            Error::CodecIncompatible(_, ctx) => ctx,
            Error::MediaResourceError(_, ctx) => ctx,
            Error::SdpError(_, ctx) => ctx,
            Error::AuthenticationFailed(_, ctx) => ctx,
            Error::AuthChallenge { context, .. } => context,
            Error::InvalidCredentials(_, ctx) => ctx,
            Error::InvalidRequest(_, ctx) => ctx,
            Error::InvalidResponse(_, ctx) => ctx,
            Error::InvalidHeader(_, ctx) => ctx,
            Error::MissingHeader(_, ctx) => ctx,
            Error::UnsupportedMethod(_, ctx) => ctx,
            Error::OperationTimeout(_, ctx) => ctx,
            Error::ResponseTimeout(_, ctx) => ctx,
            Error::DialogTimeout(_, ctx) => ctx,
            Error::ResourceAllocationFailed(_, ctx) => ctx,
            Error::ResourceLimitExceeded(_, ctx) => ctx,
            Error::MemoryAllocationFailed(_, ctx) => ctx,
            Error::TransactionError(_, ctx) => ctx,
            Error::SipError(_, ctx) => ctx,
            Error::RtpError(_, ctx) => ctx,
            Error::MediaError(_, ctx) => ctx,
            Error::IoError(_, ctx) => ctx,
            Error::ConfigurationError(_, ctx) => ctx,
            Error::InternalError(_, ctx) => ctx,
            Error::UnexpectedError(_, _, ctx) => ctx,
            Error::SerializationError(_, ctx) => ctx,
            Error::InvalidMediaState { context, .. } => context,
            Error::TransportError(error, ctx) => ctx,
            Error::Unsupported { feature, context, .. } => context,
            Error::MissingDialogData { context, .. } => context,
        }
    }

    /// Get the error category
    pub fn category(&self) -> ErrorCategory {
        self.context().category
    }

    /// Get the error severity
    pub fn severity(&self) -> ErrorSeverity {
        self.context().severity
    }

    /// Check if the error is retryable
    pub fn is_retryable(&self) -> bool {
        self.context().retryable
    }

    /// Get the suggested recovery action
    pub fn recovery_action(&self) -> &RecoveryAction {
        &self.context().recovery
    }

    /// Create a new session not found error
    pub fn session_not_found(id: &str) -> Self {
        Error::SessionNotFoundWithId(
            id.to_string(),
            ErrorContext {
                category: ErrorCategory::Session,
                severity: ErrorSeverity::Error,
                recovery: RecoveryAction::RecreateSession,
                retryable: false,
                session_id: Some(id.to_string()),
                ..Default::default()
            }
        )
    }

    /// Create a new dialog not found error
    pub fn dialog_not_found(id: &str) -> Self {
        Error::DialogNotFoundWithId(
            id.to_string(),
            ErrorContext {
                category: ErrorCategory::Dialog,
                severity: ErrorSeverity::Error,
                recovery: RecoveryAction::None,
                retryable: false,
                dialog_id: Some(id.to_string()),
                ..Default::default()
            }
        )
    }

    /// Create a transaction timeout error
    pub fn transaction_timeout(id: &str) -> Self {
        Error::TransactionTimeout(
            id.to_string(),
            ErrorContext {
                category: ErrorCategory::Timeout,
                severity: ErrorSeverity::Error,
                recovery: RecoveryAction::Retry,
                retryable: true,
                transaction_id: Some(id.to_string()),
                ..Default::default()
            }
        )
    }

    /// Create a network unreachable error
    pub fn network_unreachable(details: &str) -> Self {
        Error::NetworkUnreachable(
            details.to_string(),
            ErrorContext {
                category: ErrorCategory::Network,
                severity: ErrorSeverity::Error,
                recovery: RecoveryAction::Wait(std::time::Duration::from_secs(5)),
                retryable: true,
                ..Default::default()
            }
        )
    }

    /// Create an authentication failed error
    pub fn authentication_failed(details: &str) -> Self {
        Error::AuthenticationFailed(
            details.to_string(),
            ErrorContext {
                category: ErrorCategory::Authentication,
                severity: ErrorSeverity::Error,
                recovery: RecoveryAction::CheckConfiguration("credentials".to_string()),
                retryable: false,
                ..Default::default()
            }
        )
    }

    /// Create a transport error with context
    pub fn transport_error(error: rvoip_sip_transport::error::Error, details: &str) -> Self {
        Self::TransportError(
            error,
            ErrorContext {
                category: ErrorCategory::Network,
                severity: ErrorSeverity::Error,
                recovery: RecoveryAction::Retry,
                retryable: true,
                timestamp: std::time::SystemTime::now(),
                details: Some(details.to_string()),
                ..Default::default()
            }
        )
    }
} 