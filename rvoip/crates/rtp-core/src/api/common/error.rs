//! Error definitions
//!
//! This module defines common error types used by both client and server APIs.

use thiserror::Error;

/// Error types for media transport
#[derive(Debug, Error, Clone)]
pub enum MediaTransportError {
    /// Configuration error
    #[error("Configuration error: {0}")]
    ConfigError(String),
    
    /// Transport error
    #[error("Transport error: {0}")]
    Transport(String),
    
    /// Security error
    #[error("Security error: {0}")]
    Security(String),
    
    /// Initialization error
    #[error("Initialization error: {0}")]
    InitializationError(String),
    
    /// Connection error
    #[error("Connection error: {0}")]
    ConnectionError(String),
    
    /// Authentication error
    #[error("Authentication error: {0}")]
    AuthenticationError(String),
    
    /// Packet send error
    #[error("Send error: {0}")]
    SendError(String),
    
    /// Packet receive error
    #[error("Receive error: {0}")]
    ReceiveError(String),
    
    /// Not connected error
    #[error("Transport not connected")]
    NotConnected,
    
    /// Configuration error
    #[error("Configuration error: {0}")]
    Configuration(String),
    
    /// Timeout error
    #[error("Timeout error: {0}")]
    Timeout(String),
}

/// Error types for security operations
#[derive(Error, Debug)]
pub enum SecurityError {
    /// Failed to initialize security
    #[error("Failed to initialize security: {0}")]
    InitError(String),
    
    /// Configuration error
    #[error("Configuration error: {0}")]
    Configuration(String),
    
    /// Error during DTLS handshake
    #[error("DTLS handshake error: {0}")]
    Handshake(String),
    
    /// Handshake error (specific to handshake process)
    #[error("Handshake error: {0}")]
    HandshakeError(String),
    
    /// Internal error
    #[error("Internal security error: {0}")]
    Internal(String),
    
    /// Error during SRTP operations
    #[error("SRTP error: {0}")]
    SrtpError(String),
    
    /// Certificate error
    #[error("Certificate error: {0}")]
    CertificateError(String),
    
    /// Not initialized error
    #[error("Component not initialized: {0}")]
    NotInitialized(String),
}

/// Error types for buffer operations
#[derive(Error, Debug)]
pub enum BufferError {
    /// Buffer is full
    #[error("Buffer is full")]
    BufferFull,
    
    /// Buffer is empty
    #[error("Buffer is empty")]
    BufferEmpty,
    
    /// Configuration error
    #[error("Configuration error: {0}")]
    ConfigurationError(String),
    
    /// Other buffer operation error
    #[error("Buffer operation error: {0}")]
    OperationError(String),
}

/// Error types for statistics operations
#[derive(Error, Debug)]
pub enum StatsError {
    /// No statistics available
    #[error("No statistics available")]
    NoStatsAvailable,
    
    /// Invalid stream identifier
    #[error("Invalid stream identifier: {0}")]
    InvalidStreamId(String),
    
    /// Other statistics error
    #[error("Statistics error: {0}")]
    Other(String),
} 