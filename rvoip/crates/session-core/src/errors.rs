use thiserror::Error;

/// Errors related to session management
#[derive(Error, Debug)]
pub enum Error {
    /// Session not found
    #[error("Session not found: {0}")]
    SessionNotFound(String),

    /// Dialog not found (with identifier)
    #[error("Dialog not found: {0}")]
    DialogNotFoundWithId(String),

    /// Dialog not found (generic)
    #[error("Dialog not found")]
    DialogNotFound,

    /// Dialog already exists
    #[error("Dialog already exists")]
    DialogAlreadyExists,

    /// Invalid state transition
    #[error("Invalid state transition: {0} -> {1}")]
    InvalidStateTransition(String, String),

    /// Invalid dialog state
    #[error("Invalid dialog state: {0}")]
    InvalidDialogState(String),

    /// Invalid session state
    #[error("Invalid session state: {0}")]
    InvalidSessionState(String),

    /// Session terminated
    #[error("Session terminated")]
    SessionTerminated,

    /// Media-related errors
    #[error("Media negotiation error: {0}")]
    MediaNegotiationError(String),

    /// Media error (string description)
    #[error("Media error: {0}")]
    MediaError(String),

    /// Media error (from media-core)
    #[error("Media processing error: {0}")]
    Media(#[from] rvoip_media_core::Error),

    /// Request-related errors
    #[error("Invalid request: {0}")]
    InvalidRequest(String),

    /// Timeout-related errors
    #[error("Timeout: {0}")]
    Timeout(String),

    /// External dependency errors
    #[error("Transaction error: {0}")]
    TransactionError(#[from] rvoip_transaction_core::Error),

    #[error("SIP message error: {0}")]
    SipError(#[from] rvoip_sip_core::Error),

    #[error("RTP error: {0}")]
    RtpError(#[from] rvoip_rtp_core::Error),

    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    /// Other errors
    #[error("General error: {0}")]
    General(String),

    #[error("Other error: {0}")]
    Other(String),
} 