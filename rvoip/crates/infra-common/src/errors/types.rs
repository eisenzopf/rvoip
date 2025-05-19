use thiserror::Error;
use std::io;

/// Result type alias using our custom Error type
pub type Result<T> = std::result::Result<T, Error>;

/// Common error types for the RVOIP stack
#[derive(Error, Debug)]
pub enum Error {
    #[error("I/O error: {0}")]
    Io(#[from] io::Error),
    
    #[error("Configuration error: {0}")]
    Config(String),
    
    #[error("Component error: {0}")]
    Component(String),
    
    #[error("Component not found: {0}")]
    ComponentNotFound(String),
    
    #[error("Component not ready: {0}")]
    ComponentNotReady(String),
    
    #[error("Dependency error: {0}")]
    Dependency(String),
    
    #[error("Event error: {0}")]
    Event(String),
    
    #[error("Timeout error: {0}")]
    Timeout(String),
    
    #[error("Initialization error: {0}")]
    Initialization(String),
    
    #[error("Validation error: {0}")]
    Validation(String),
    
    #[error("Resource not found: {0}")]
    NotFound(String),
    
    #[error("Protocol error: {0}")]
    Protocol(String),
    
    #[error("Internal error: {0}")]
    Internal(String),
    
    #[error("External service error: {0}")]
    ExternalService(String),
    
    #[error("{0}")]
    Custom(String),
    
    #[error("Unknown error")]
    Unknown,
}

/// Trait for converting domain-specific errors to common errors
pub trait ErrorMapper<E> {
    /// Map a domain-specific error to a common error
    fn map_error(error: E) -> Error;
} 