//! Client API
//!
//! This module provides client-specific implementations and interfaces for media transport.

pub mod transport;
pub mod security;
pub mod config;

// Re-export key client types for convenience
pub use self::transport::MediaTransportClient;
pub use self::security::ClientSecurityContext;
pub use self::config::ClientConfig; 