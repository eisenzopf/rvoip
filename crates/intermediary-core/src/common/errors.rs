//! Error types for the intermediary-core library

use thiserror::Error;

#[derive(Error, Debug)]
pub enum IntermediaryError {
    #[error("Routing failed: {0}")]
    RoutingError(String),

    #[error("Policy violation: {0}")]
    PolicyViolation(String),

    #[error("Session not found: {0}")]
    SessionNotFound(String),

    #[error("Invalid mode for operation: {0}")]
    InvalidMode(String),

    #[error("Bridge operation failed: {0}")]
    BridgeError(String),

    #[error("Configuration error: {0}")]
    ConfigError(String),

    #[error("Session core error: {0}")]
    SessionCoreError(#[from] rvoip_session_core_v2::errors::SessionError),

    #[error("Other error: {0}")]
    Other(String),
}

pub type Result<T> = std::result::Result<T, IntermediaryError>;