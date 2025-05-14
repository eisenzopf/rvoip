use crate::transaction::{TransactionKey, TransactionKind, TransactionState};
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
    #[error("SIP transport error: {source}")]
    TransportError { 
        #[source]
        source: TransportErrorWrapper,
        context: Option<String>, 
    },

    /// Transaction not found for the given key.
    #[error("Transaction not found: {key} (context: {context})")]
    TransactionNotFound { 
        key: TransactionKey,
        context: String,
    },

    /// Transaction with the given key already exists.
    #[error("Transaction already exists: {key} (kind: {kind:?})")]
    TransactionExists { 
        key: TransactionKey,
        kind: TransactionKind,
    },

    /// Invalid transaction state transition attempted.
    #[error("Invalid state transition: {from_state:?} -> {to_state:?} for {transaction_kind:?} transaction")]
    InvalidStateTransition {
        transaction_kind: TransactionKind,
        from_state: TransactionState,
        to_state: TransactionState,
        transaction_id: Option<TransactionKey>,
    },

    /// Transaction timed out (specific timers T_B, T_F, T_H).
    #[error("Transaction timed out: {key} (timer: {timer})")]
    TransactionTimeout {
        key: TransactionKey,
        timer: String,
    },
    
    /// Timer error
    #[error("Timer error: {message}")]
    TimerError {
        message: String,
    },

    /// I/O error.
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),

    /// Internal channel error (e.g., receiver dropped).
    #[error("Channel error: {context}")]
    ChannelError {
        context: String,
    },
    
    /// Transaction creation error
    #[error("Failed to create transaction: {message}")]
    TransactionCreationError {
        message: String,
    },
    
    /// Transaction message processing error
    #[error("Failed to process message: {message} for transaction {transaction_id:?}")]
    MessageProcessingError {
        message: String,
        transaction_id: Option<TransactionKey>,
    },

    /// Other miscellaneous errors.
    #[error("Other error: {0}")]
    Other(String),
}

/// Wrapper for transport errors to provide consistent Debug/Display
#[derive(Debug)]
pub struct TransportErrorWrapper(pub String);

impl std::fmt::Display for TransportErrorWrapper {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl std::error::Error for TransportErrorWrapper {}

// Manual From impl for transport errors
impl From<rvoip_sip_transport::Error> for Error {
    fn from(e: rvoip_sip_transport::Error) -> Self {
        Error::TransportError { 
            source: TransportErrorWrapper(e.to_string()),
            context: None,
        }
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

// More specific error for channel errors
impl<T> From<tokio::sync::mpsc::error::SendError<T>> for Error {
    fn from(e: tokio::sync::mpsc::error::SendError<T>) -> Self {
        Error::ChannelError {
            context: format!("Send error: channel closed while sending {:?}", std::any::type_name::<T>()),
        }
    }
}

// Add helper methods to create more specific errors with context
impl Error {
    /// Create a new TransactionNotFound error with context
    pub fn transaction_not_found(key: TransactionKey, context: impl Into<String>) -> Self {
        Error::TransactionNotFound { 
            key, 
            context: context.into(),
        }
    }
    
    /// Create a new TransportError with context
    pub fn transport_error(source: rvoip_sip_transport::Error, context: impl Into<String>) -> Self {
        Error::TransportError { 
            source: TransportErrorWrapper(source.to_string()),
            context: Some(context.into()),
        }
    }
    
    /// Create a new InvalidStateTransition error
    pub fn invalid_state_transition(
        transaction_kind: TransactionKind,
        from_state: TransactionState,
        to_state: TransactionState,
        transaction_id: Option<TransactionKey>,
    ) -> Self {
        Error::InvalidStateTransition {
            transaction_kind,
            from_state,
            to_state,
            transaction_id,
        }
    }
    
    /// Create a new ChannelError with context
    pub fn channel_error(context: impl Into<String>) -> Self {
        Error::ChannelError {
            context: context.into(),
        }
    }
    
    /// Create a new TransactionTimeout error
    pub fn transaction_timeout(key: TransactionKey, timer: impl Into<String>) -> Self {
        Error::TransactionTimeout {
            key,
            timer: timer.into(),
        }
    }
} 