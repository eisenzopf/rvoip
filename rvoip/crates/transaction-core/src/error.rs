use std::io;
use thiserror::Error;

/// A type alias for handling `Result`s with `Error`
pub type Result<T> = std::result::Result<T, Error>;

/// Errors that can occur in SIP transaction handling
#[derive(Error, Debug)]
pub enum Error {
    /// Error in SIP message processing
    #[error("SIP message error: {0}")]
    SipMessageError(#[from] rvoip_sip_core::Error),

    /// Error in SIP transport
    #[error("SIP transport error: {0}")]
    TransportError(#[from] rvoip_sip_transport::Error),

    /// Transaction not found
    #[error("Transaction not found: {0}")]
    TransactionNotFound(String),

    /// Transaction already exists
    #[error("Transaction already exists: {0}")]
    TransactionExists(String),

    /// Invalid transaction state transition
    #[error("Invalid transaction state transition: {0}")]
    InvalidStateTransition(String),

    /// Transaction timeout
    #[error("Transaction timed out: {0}")]
    TransactionTimeout(String),

    /// I/O error
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    /// Channel error (receiver dropped)
    #[error("Channel closed")]
    ChannelClosed,

    /// Other error
    #[error("{0}")]
    Other(String),
}

impl From<&str> for Error {
    fn from(s: &str) -> Self {
        Error::Other(s.to_string())
    }
}

impl From<String> for Error {
    fn from(s: String) -> Self {
        Error::Other(s)
    }
}

impl<T> From<tokio::sync::mpsc::error::SendError<T>> for Error {
    fn from(_: tokio::sync::mpsc::error::SendError<T>) -> Self {
        Error::ChannelClosed
    }
} 