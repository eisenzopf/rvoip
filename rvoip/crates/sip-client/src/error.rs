use std::io;
use std::fmt;

use thiserror::Error;

use rvoip_sip_core::{Error as SipError};
use rvoip_transaction_core::{Error as TransactionError};
use rvoip_media_core::{Error as MediaError};

/// Result type for sip-client operations
pub type Result<T> = std::result::Result<T, Error>;

/// Error types for the SIP client
#[derive(Error, Debug)]
pub enum Error {
    /// SIP protocol errors
    #[error("SIP error: {0}")]
    Sip(#[from] SipError),

    /// Transaction errors
    #[error("Transaction error: {0}")]
    Transaction(#[from] TransactionError),

    /// Media errors
    #[error("Media error: {0}")]
    Media(#[from] MediaError),

    /// I/O errors
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    /// Invalid arguments
    #[error("Invalid argument: {0}")]
    InvalidArgument(String),

    /// Invalid state 
    #[error("Invalid state: {0}")]
    InvalidState(String),

    /// Call not found
    #[error("Call not found: {0}")]
    CallNotFound(String),
    
    /// Network errors
    #[error("Network error: {0}")]
    Network(String),
    
    /// Authentication errors
    #[error("Authentication error: {0}")]
    Auth(String),
    
    /// Feature not available
    #[error("Feature not available: {0}")]
    FeatureNotAvailable(String),
    
    /// Other error
    #[error("{0}")]
    Other(String),
}

impl From<anyhow::Error> for Error {
    fn from(err: anyhow::Error) -> Self {
        Error::Other(err.to_string())
    }
}

impl From<&str> for Error {
    fn from(s: &str) -> Self {
        Error::Other(s.to_string())
    }
} 