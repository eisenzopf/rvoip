//! Error and `Result` types for the `rvoip-sip` session layer.
//!
//! [`SessionError`] is the crate-wide error enum returned by the public API
//! surfaces (`Endpoint`, `StreamPeer`, `CallbackPeer`, `UnifiedCoordinator`,
//! `SessionHandle`). [`Result`] is the `Result<T, SessionError>` alias used
//! throughout the crate.

use thiserror::Error;

/// Convenience alias for `Result<T, SessionError>` used across the crate's API.
pub type Result<T> = std::result::Result<T, SessionError>;

/// Errors returned by the `rvoip-sip` session layer.
#[derive(Debug, Error)]
pub enum SessionError {
    /// No session with the given identifier exists in the registry.
    #[error("Session not found: {0}")]
    SessionNotFound(String),

    /// A requested state-machine transition is not legal from the current state.
    #[error("Invalid state transition: {0}")]
    InvalidTransition(String),

    /// An error originating in the dialog layer (`rvoip-sip-dialog`).
    #[error("Dialog error: {0}")]
    DialogError(String),

    /// An error originating in the media layer (`rvoip-media-core`).
    #[error("Media error: {0}")]
    MediaError(String),

    /// SIP signalling succeeded but wiring media to the negotiated session failed.
    #[error("Media integration error: {reason}")]
    MediaIntegration {
        /// Human-readable description of the media-integration failure.
        reason: String,
    },

    /// SDP offer/answer negotiation failed (no common codec, malformed SDP, etc.).
    #[error("SDP negotiation failed: {0}")]
    SDPNegotiationFailed(String),

    /// Invalid or inconsistent configuration supplied to a builder or coordinator.
    #[error("Configuration error: {0}")]
    ConfigurationError(String),

    /// Configuration error (legacy alias of [`SessionError::ConfigurationError`]).
    #[error("Config error: {0}")]
    ConfigError(String),

    /// An application-supplied argument was malformed or out of range.
    #[error("Invalid input: {0}")]
    InvalidInput(String),

    /// An operation did not complete within its allotted time.
    #[error("Timeout: {0}")]
    Timeout(String),

    /// A transport-level network error occurred.
    #[error("Network error: {0}")]
    NetworkError(String),

    /// A SIP protocol violation was detected.
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
    /// `control.invite(...).with_credentials(...)` (per-call).
    #[error("server challenged INVITE but no credentials are on file")]
    MissingCredentialsForInviteAuth,

    /// RFC 3261 §22.2 — INVITE auth has already been retried once and the
    /// server challenged again. Prevents loops against a broken server or
    /// wrong credentials.
    #[error("INVITE auth retry limit exceeded")]
    InviteAuthRetryExhausted,

    /// An unexpected internal invariant was violated.
    #[error("Internal error: {0}")]
    InternalError(String),

    /// An underlying `std::io` error (transport sockets, file I/O).
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    /// The requested capability is recognized but not yet implemented.
    #[error("Not implemented: {0}")]
    NotImplemented(String),

    /// A call transfer (REFER flow) failed.
    #[error("Transfer failed: {0}")]
    TransferFailed(String),

    /// Authentication failed or could not be completed.
    #[error("Authentication error: {0}")]
    AuthError(String),

    /// A REGISTER flow failed after any supported retry path.
    #[error("Registration failed: {0}")]
    RegistrationFailed(String),

    /// A flattened/stringly error from a lower layer that has no dedicated variant.
    #[error("Other error: {0}")]
    Other(String),

    /// SIP_API_DESIGN_2 §8 — a builder setter or `with_header` call
    /// staged a header that violates the per-method policy. The most
    /// common case is staging a stack-managed name (Call-ID, CSeq,
    /// Via, Max-Forwards) or a method-shaped name that has a
    /// dedicated setter (e.g. Authorization → `with_credentials`).
    #[error("header policy violation on {method}: {header} — {reason}")]
    HeaderPolicy {
        /// SIP method whose per-method header policy was violated.
        method: rvoip_sip_core::Method,
        /// The offending header name.
        header: rvoip_sip_core::types::headers::HeaderName,
        /// Why the header was rejected.
        reason: crate::api::headers::ViolationReason,
    },

    /// SIP_API_DESIGN_2 §8 — `HeaderPolicy::validate_outbound`
    /// reported one or more required application-supplied headers
    /// were missing for the chosen method.
    #[error("required application header(s) missing for {method}: {names:?}")]
    MissingRequiredHeader {
        /// SIP method that requires the missing header(s).
        method: rvoip_sip_core::Method,
        /// The required header names that were not supplied.
        names: Vec<rvoip_sip_core::types::headers::HeaderName>,
    },

    /// SIP_API_DESIGN_2 §7.3 invariant #5 — a second `.send()` was
    /// attempted on the same session for a method whose
    /// `pending_<method>_options` stash slot is still occupied by an
    /// in-flight prior `.send()`. Wait for the first future to
    /// complete (or drop cleanly) before starting another of the
    /// same method.
    #[error("another {method} is already in flight on this session")]
    Conflict {
        /// SIP method whose in-flight `.send()` blocks a second concurrent send.
        method: rvoip_sip_core::Method,
    },
}

impl From<crate::api::headers::HeaderPolicyViolation> for SessionError {
    fn from(v: crate::api::headers::HeaderPolicyViolation) -> Self {
        SessionError::HeaderPolicy {
            method: v.method,
            header: v.header,
            reason: v.reason,
        }
    }
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
