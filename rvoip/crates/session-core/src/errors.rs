use std::fmt;
use thiserror::Error;
use rvoip_dialog_core::DialogError;

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
    
    /// Wait and retry after specified duration
    WaitAndRetry(std::time::Duration),
    
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
            RecoveryAction::WaitAndRetry(duration) => 
                write!(f, "Wait and retry after {:?} before retrying", duration),
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
    
    /// Builder for session-specific error context
    pub fn session(session_id: &str) -> SessionErrorContextBuilder {
        SessionErrorContextBuilder {
            context: ErrorContext {
                category: ErrorCategory::Session,
                session_id: Some(session_id.to_string()),
                timestamp: std::time::SystemTime::now(),
                ..Default::default()
            }
        }
    }
    
    /// Builder for dialog-specific error context
    pub fn dialog(dialog_id: &str) -> DialogErrorContextBuilder {
        DialogErrorContextBuilder {
            context: ErrorContext {
                category: ErrorCategory::Dialog,
                dialog_id: Some(dialog_id.to_string()),
                timestamp: std::time::SystemTime::now(),
                ..Default::default()
            }
        }
    }
    
    /// Builder for resource-specific error context
    pub fn resource(resource_name: &str) -> ResourceErrorContextBuilder {
        ResourceErrorContextBuilder {
            context: ErrorContext {
                category: ErrorCategory::Resource,
                details: Some(format!("Resource: {}", resource_name)),
                timestamp: std::time::SystemTime::now(),
                ..Default::default()
            }
        }
    }
}

/// Builder for session-specific error contexts
pub struct SessionErrorContextBuilder {
    context: ErrorContext,
}

impl SessionErrorContextBuilder {
    /// Add session state information
    pub fn with_state(mut self, state: &str) -> Self {
        if let Some(ref mut details) = self.context.details {
            *details = format!("{}, state: {}", details, state);
        } else {
            self.context.details = Some(format!("state: {}", state));
        }
        self
    }
    
    /// Add dialog ID
    pub fn with_dialog(mut self, dialog_id: &str) -> Self {
        self.context.dialog_id = Some(dialog_id.to_string());
        self
    }
    
    /// Add media session information
    pub fn with_media_session(mut self, media_session_id: &str) -> Self {
        if let Some(ref mut details) = self.context.details {
            *details = format!("{}, media_session: {}", details, media_session_id);
        } else {
            self.context.details = Some(format!("media_session: {}", media_session_id));
        }
        self
    }
    
    /// Add session duration information
    pub fn with_duration(mut self, duration: std::time::Duration) -> Self {
        if let Some(ref mut details) = self.context.details {
            *details = format!("{}, duration: {:?}", details, duration);
        } else {
            self.context.details = Some(format!("duration: {:?}", duration));
        }
        self
    }
    
    /// Set error severity
    pub fn severity(mut self, severity: ErrorSeverity) -> Self {
        self.context.severity = severity;
        self
    }
    
    /// Set recovery action
    pub fn recovery(mut self, action: RecoveryAction) -> Self {
        self.context.recovery = action;
        self
    }
    
    /// Mark as retryable
    pub fn retryable(mut self) -> Self {
        self.context.retryable = true;
        self
    }
    
    /// Add detailed message
    pub fn message(mut self, message: &str) -> Self {
        if let Some(ref mut details) = self.context.details {
            *details = format!("{}, {}", details, message);
        } else {
            self.context.details = Some(message.to_string());
        }
        self
    }
    
    /// Build the error context
    pub fn build(self) -> ErrorContext {
        self.context
    }
}

/// Builder for dialog-specific error contexts
pub struct DialogErrorContextBuilder {
    context: ErrorContext,
}

impl DialogErrorContextBuilder {
    /// Add session ID
    pub fn with_session(mut self, session_id: &str) -> Self {
        self.context.session_id = Some(session_id.to_string());
        self
    }
    
    /// Add transaction ID
    pub fn with_transaction(mut self, transaction_id: &str) -> Self {
        self.context.transaction_id = Some(transaction_id.to_string());
        self
    }
    
    /// Set error severity
    pub fn severity(mut self, severity: ErrorSeverity) -> Self {
        self.context.severity = severity;
        self
    }
    
    /// Set recovery action
    pub fn recovery(mut self, action: RecoveryAction) -> Self {
        self.context.recovery = action;
        self
    }
    
    /// Add detailed message
    pub fn message(mut self, message: &str) -> Self {
        if let Some(ref mut details) = self.context.details {
            *details = format!("{}, {}", details, message);
        } else {
            self.context.details = Some(message.to_string());
        }
        self
    }
    
    /// Build the error context
    pub fn build(self) -> ErrorContext {
        self.context
    }
}

/// Builder for resource-specific error contexts
pub struct ResourceErrorContextBuilder {
    context: ErrorContext,
}

impl ResourceErrorContextBuilder {
    /// Add session ID
    pub fn with_session(mut self, session_id: &str) -> Self {
        self.context.session_id = Some(session_id.to_string());
        self
    }
    
    /// Add current and limit values
    pub fn with_limits(mut self, current: usize, limit: usize) -> Self {
        if let Some(ref mut details) = self.context.details {
            *details = format!("{}, usage: {}/{}", details, current, limit);
        } else {
            self.context.details = Some(format!("usage: {}/{}", current, limit));
        }
        self
    }
    
    /// Set error severity
    pub fn severity(mut self, severity: ErrorSeverity) -> Self {
        self.context.severity = severity;
        self
    }
    
    /// Set recovery action
    pub fn recovery(mut self, action: RecoveryAction) -> Self {
        self.context.recovery = action;
        self
    }
    
    /// Add detailed message
    pub fn message(mut self, message: &str) -> Self {
        if let Some(ref mut details) = self.context.details {
            *details = format!("{}, {}", details, message);
        } else {
            self.context.details = Some(message.to_string());
        }
        self
    }
    
    /// Build the error context
    pub fn build(self) -> ErrorContext {
        self.context
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

    /// Malformed request
    #[error("Malformed request: {0}")]
    MalformedRequest(String, ErrorContext),

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

    /// Resource limit exceeded with detailed information
    #[error("Resource limit exceeded for {resource}: {current}/{limit}")]
    ResourceLimitExceededDetailed {
        /// The resource that exceeded its limit
        resource: String,
        /// The limit that was exceeded
        limit: usize,
        /// Current usage
        current: usize,
        /// Error context
        context: ErrorContext,
    },

    /// Memory allocation failed
    #[error("Memory allocation failed: {0}")]
    MemoryAllocationFailed(String, ErrorContext),

    //
    // External dependency errors
    //
    
    /// Dialog layer error (for dialog-core integration)
    #[error("Dialog layer error: {0}")]
    DialogError(
        String, 
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

    /// Network error (replaces transport error for architectural compliance)
    #[error("Network error: {0}")]
    NetworkError(String, ErrorContext),

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
            Error::MalformedRequest(_, ctx) => ctx,
            Error::InvalidResponse(_, ctx) => ctx,
            Error::InvalidHeader(_, ctx) => ctx,
            Error::MissingHeader(_, ctx) => ctx,
            Error::UnsupportedMethod(_, ctx) => ctx,
            Error::OperationTimeout(_, ctx) => ctx,
            Error::ResponseTimeout(_, ctx) => ctx,
            Error::DialogTimeout(_, ctx) => ctx,
            Error::ResourceAllocationFailed(_, ctx) => ctx,
            Error::ResourceLimitExceeded(_, context) => context,
            Error::ResourceLimitExceededDetailed { context, .. } => context,
            Error::MemoryAllocationFailed(_, context) => context,
            Error::DialogError(_, ctx) => ctx,
            Error::SipError(_, ctx) => ctx,
            Error::RtpError(_, ctx) => ctx,
            Error::MediaError(_, ctx) => ctx,
            Error::IoError(_, ctx) => ctx,
            Error::ConfigurationError(_, ctx) => ctx,
            Error::InternalError(_, ctx) => ctx,
            Error::UnexpectedError(_, _, ctx) => ctx,
            Error::SerializationError(_, ctx) => ctx,
            Error::InvalidMediaState { context, .. } => context,
            Error::NetworkError(_, ctx) => ctx,
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

    /// Create a network error with context
    pub fn network_error(details: &str) -> Self {
        Error::NetworkError(
            details.to_string(),
            ErrorContext {
                category: ErrorCategory::Network,
                severity: ErrorSeverity::Error,
                recovery: RecoveryAction::Retry,
                retryable: true,
                ..Default::default()
            }
        )
    }

    /// Create a session error with rich context
    pub fn session_error(session_id: &str, message: &str) -> Self {
        Error::InternalError(
            message.to_string(),
            ErrorContext::session(session_id)
                .message(message)
                .severity(ErrorSeverity::Error)
                .build()
        )
    }
    
    /// Create a session state transition error with rich context
    pub fn session_state_error(session_id: &str, from_state: &str, to_state: &str, reason: &str) -> Self {
        Error::InvalidSessionStateTransition {
            from: from_state.to_string(),
            to: to_state.to_string(),
            context: ErrorContext::session(session_id)
                .with_state(from_state)
                .message(&format!("Cannot transition from {} to {}: {}", from_state, to_state, reason))
                .severity(ErrorSeverity::Error)
                .recovery(RecoveryAction::CheckConfiguration("session_state".to_string()))
                .build()
        }
    }
    
    /// Create a session timeout error with context
    pub fn session_timeout(session_id: &str, timeout_duration: std::time::Duration) -> Self {
        Error::OperationTimeout(
            format!("Session {} timed out after {:?}", session_id, timeout_duration),
            ErrorContext::session(session_id)
                .with_duration(timeout_duration)
                .message("Session operation timed out")
                .severity(ErrorSeverity::Warning)
                .recovery(RecoveryAction::RetryWithBackoff(std::time::Duration::from_secs(5)))
                .retryable()
                .build()
        )
    }
    
    /// Create a media session error with context
    pub fn media_session_error(session_id: &str, media_session_id: &str, details: &str) -> Self {
        Error::MediaStreamError(
            details.to_string(),
            ErrorContext::session(session_id)
                .with_media_session(media_session_id)
                .message(details)
                .severity(ErrorSeverity::Error)
                .recovery(RecoveryAction::RecreateSession)
                .build()
        )
    }
    
    /// Create a dialog error with rich context
    pub fn dialog_error(dialog_id: &str, session_id: Option<&str>, message: &str) -> Self {
        let mut builder = ErrorContext::dialog(dialog_id)
            .message(message)
            .severity(ErrorSeverity::Error);
            
        if let Some(sid) = session_id {
            builder = builder.with_session(sid);
        }
        
        Error::DialogError(
            message.to_string(),
            builder.build()
        )
    }
    
    /// Create a resource limit error with detailed context
    pub fn resource_limit_error(resource_name: &str, current: usize, limit: usize, session_id: Option<&str>) -> Self {
        let mut builder = ErrorContext::resource(resource_name)
            .with_limits(current, limit)
            .severity(ErrorSeverity::Error)
            .recovery(RecoveryAction::WaitAndRetry(std::time::Duration::from_secs(30)));
            
        if let Some(sid) = session_id {
            builder = builder.with_session(sid);
        }
        
        Error::ResourceLimitExceededDetailed {
            resource: resource_name.to_string(),
            limit,
            current,
            context: builder.build()
        }
    }
    
    /// Create a configuration error with helpful context
    pub fn config_error(parameter: &str, details: &str) -> Self {
        Error::ConfigurationError(
            details.to_string(),
            ErrorContext {
                category: ErrorCategory::Configuration,
                severity: ErrorSeverity::Error,
                recovery: RecoveryAction::CheckConfiguration(parameter.to_string()),
                retryable: false,
                details: Some(format!("Configuration parameter '{}': {}", parameter, details)),
                timestamp: std::time::SystemTime::now(),
                ..Default::default()
            }
        )
    }
}

impl From<DialogError> for Error {
    fn from(dialog_error: DialogError) -> Self {
        Error::InternalError(
            format!("Dialog error: {}", dialog_error),
            ErrorContext::default().with_message("Dialog layer error")
        )
    }
}

impl From<rvoip_sip_core::Error> for Error {
    fn from(sip_error: rvoip_sip_core::Error) -> Self {
        Error::InternalError(
            format!("SIP core error: {}", sip_error),
            ErrorContext::default().with_message("SIP layer error")
        )
    }
} 