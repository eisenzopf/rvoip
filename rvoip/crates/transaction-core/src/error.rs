use crate::transaction::TransactionKey;
use std::io;
use thiserror::Error;

/// A type alias for handling `Result`s with `Error`
pub type Result<T> = std::result::Result<T, Error>;

/// Errors that can occur in SIP transaction handling
#[derive(Error, Debug)]
pub enum Error {
    /// Error originating from the sip-core crate (parsing, building messages, etc.)
    #[error("SIP core error: {0}")]
    SipCoreError(#[from] rvoip_sip_core::Error),

    /// Error originating from the sip-transport crate.
    #[error("SIP transport error: {0}")]
    TransportError(String), // Transport errors might not be easily cloneable/static

    /// Transaction not found for the given key.
    #[error("Transaction not found: {0:?}")]
    TransactionNotFound(TransactionKey),

    /// Transaction with the given key already exists.
    #[error("Transaction already exists: {0:?}")]
    TransactionExists(TransactionKey),

    /// Invalid transaction state transition attempted.
    #[error("Invalid transaction state transition: {0}")]
    InvalidStateTransition(String),

    /// Transaction timed out (specific timers T_B, T_F, T_H).
    #[error("Transaction timed out: {0}")]
    TransactionTimeout(TransactionKey),

    /// I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    /// Internal channel error (e.g., receiver dropped).
    #[error("Internal channel closed")]
    ChannelClosed,

    /// Other miscellaneous errors.
    #[error("Other error: {0}")]
    Other(String),
}

// Manual From impl for transport errors if needed
impl From<rvoip_sip_transport::Error> for Error {
    fn from(e: rvoip_sip_transport::Error) -> Self {
        Error::TransportError(e.to_string()) // Convert to String
    }
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

// Blanket implementation for SendError might be too broad,
// be specific if possible, or handle where the send error occurs.
impl<T> From<tokio::sync::mpsc::error::SendError<T>> for Error {
    fn from(_: tokio::sync::mpsc::error::SendError<T>) -> Self {
        Error::ChannelClosed
    }
} 