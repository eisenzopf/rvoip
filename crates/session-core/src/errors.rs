use thiserror::Error;

/// Result type for session operations
pub type Result<T> = std::result::Result<T, SessionError>;

/// Session-related errors
#[derive(Debug, Error)]
pub enum SessionError {
    #[error("Session not found: {0}")]
    SessionNotFound(String),

    #[error("Invalid state transition: {0}")]
    InvalidTransition(String),

    #[error("Dialog error: {0}")]
    DialogError(String),

    #[error("Media error: {0}")]
    MediaError(String),

    #[error("Media integration error: {reason}")]
    MediaIntegration { reason: String },

    #[error("SDP negotiation failed: {0}")]
    SDPNegotiationFailed(String),

    #[error("Configuration error: {0}")]
    ConfigurationError(String),

    #[error("Config error: {0}")]
    ConfigError(String),

    #[error("Invalid input: {0}")]
    InvalidInput(String),

    #[error("Timeout: {0}")]
    Timeout(String),

    #[error("Network error: {0}")]
    NetworkError(String),

    #[error("Protocol error: {0}")]
    ProtocolError(String),

    /// RFC 3262 — the remote peer did not advertise `Supported: 100rel` on the
    /// INVITE, so we cannot send a reliable 183 Session Progress. Raised by
    /// `send_early_media`. Today we fail fast; a future `send_progress(sdp)`
    /// API could fall back to an unreliable 183.
    #[error("peer did not advertise 100rel; cannot send reliable 183")]
    UnreliableProvisionalsNotSupported,

    /// RFC 3261 §22.2 — the server challenged our INVITE with 401/407 but the
    /// session has no credentials on file. Set credentials via
    /// `StreamPeerBuilder::with_credentials` (per-peer default) or
    /// `PeerControl::call_with_auth` (per-call).
    #[error("server challenged INVITE but no credentials are on file")]
    MissingCredentialsForInviteAuth,

    /// RFC 3261 §22.2 — INVITE auth has already been retried once and the
    /// server challenged again. Prevents loops against a broken server or
    /// wrong credentials.
    #[error("INVITE auth retry limit exceeded")]
    InviteAuthRetryExhausted,

    #[error("Internal error: {0}")]
    InternalError(String),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    #[error("Not implemented: {0}")]
    NotImplemented(String),

    #[error("Transfer failed: {0}")]
    TransferFailed(String),

    #[error("Authentication error: {0}")]
    AuthError(String),

    #[error("Registration failed: {0}")]
    RegistrationFailed(String),

    #[error("Other error: {0}")]
    Other(String),
}

impl SessionError {
    /// True if this error means "the session is already gone from the
    /// registry" — covers both the typed `SessionNotFound` variant and the
    /// stringly-wrapped `Other("Session not found: …")` form that falls out
    /// of the `From<Box<dyn Error>>` flatteners above.
    ///
    /// Useful for fire-and-forget teardown paths (e.g. `SessionHandle::hangup`)
    /// that race against a natural call-ended cleanup: if the race is lost,
    /// the goal is already achieved and the error should be silent.
    pub fn is_session_gone(&self) -> bool {
        matches!(self, SessionError::SessionNotFound(_))
            || matches!(self, SessionError::Other(msg) if msg.starts_with("Session not found"))
            || matches!(self, SessionError::Other(msg) if msg.starts_with("Session ") && msg.ends_with(" not found"))
    }
}

impl From<Box<dyn std::error::Error>> for SessionError {
    fn from(err: Box<dyn std::error::Error>) -> Self {
        SessionError::Other(err.to_string())
    }
}

impl From<Box<dyn std::error::Error + Send + Sync>> for SessionError {
    fn from(err: Box<dyn std::error::Error + Send + Sync>) -> Self {
        SessionError::Other(err.to_string())
    }
}

impl From<rvoip_auth_core::AuthError> for SessionError {
    fn from(err: rvoip_auth_core::AuthError) -> Self {
        SessionError::AuthError(err.to_string())
    }
}
