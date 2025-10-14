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
    
    #[error("Internal error: {0}")]
    InternalError(String),
    
    #[error("IO error: {0}")]
    IoError(#[from] std::io::Error),
    
    #[error("Not implemented: {0}")]
    NotImplemented(String),
    
    #[error("Transfer failed: {0}")]
    TransferFailed(String),
    
    #[error("Other error: {0}")]
    Other(String),
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