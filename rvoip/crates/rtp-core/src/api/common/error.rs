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
    
    /// RTCP error
    #[error("RTCP error: {0}")]
    RtcpError(String),
    
    /// No clients connected
    #[error("No clients connected")]
    NoClients,
    
    /// Client not found
    #[error("Client not found: {0}")]
    ClientNotFound(String),
    
    /// Client not connected
    #[error("Client not connected: {0}")]
    ClientNotConnected(String),
}

/// Error related to security operations
#[derive(Debug, thiserror::Error)]
pub enum SecurityError {
    /// Configuration error
    #[error("Security configuration error: {0}")]
    Configuration(String),
    
    /// DTLS handshake error
    #[error("DTLS handshake error: {0}")]
    Handshake(String),
    
    /// Handshake verification error (e.g. fingerprint mismatch)
    #[error("Handshake verification error: {0}")]
    HandshakeVerification(String),
    
    /// Generic handshake error
    #[error("Handshake error: {0}")]
    HandshakeError(String),
    
    /// Not initialized error
    #[error("Security not initialized: {0}")]
    NotInitialized(String),
    
    /// Internal security error
    #[error("Internal security error: {0}")]
    Internal(String),
    
    /// Security timeout error
    #[error("Security timeout: {0}")]
    Timeout(String),
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