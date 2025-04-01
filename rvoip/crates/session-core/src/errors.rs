use thiserror::Error;

/// Errors related to session management
#[derive(Error, Debug)]
pub enum Error {
    /// Session not found
    #[error("Session not found: {0}")]
    SessionNotFound(String),

    /// Dialog not found
    #[error("Dialog not found: {0}")]
    DialogNotFound(String),

    /// Invalid state transition
    #[error("Invalid state transition: {0} -> {1}")]
    InvalidStateTransition(String, String),

    /// Media negotiation error
    #[error("Media negotiation error: {0}")]
    MediaNegotiationError(String),

    /// Transaction error
    #[error("Transaction error: {0}")]
    TransactionError(#[from] rvoip_transaction_core::Error),

    /// SIP message error
    #[error("SIP message error: {0}")]
    SipError(#[from] rvoip_sip_core::Error),

    /// RTP error
    #[error("RTP error: {0}")]
    RtpError(#[from] rvoip_rtp_core::Error),

    /// Media error
    #[error("Media error: {0}")]
    MediaError(String),

    /// IO error
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),

    /// General error
    #[error("General error: {0}")]
    General(String),
} 