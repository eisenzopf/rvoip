//! Server security API
//!
//! This module provides security-related interfaces for the server-side media transport.

use std::net::SocketAddr;
use std::sync::Arc;
use async_trait::async_trait;
use tokio::net::UdpSocket;
use tokio::sync::Mutex;

use crate::api::common::error::SecurityError;
use crate::api::common::config::{SecurityInfo, SecurityMode, SrtpProfile};

pub mod server_security_impl;

// Re-export public implementation
pub use server_security_impl::DefaultServerSecurityContext;
pub use server_security_impl::DefaultClientSecurityContext as ServerClientSecurityContext;

// Define our own types for API compatibility
/// Socket handle for network operations
#[derive(Clone)]
pub struct SocketHandle {
    /// The underlying UDP socket
    pub socket: Arc<UdpSocket>,
    /// The remote address
    pub remote_addr: Option<SocketAddr>,
}

/// DTLS connection configuration
#[derive(Clone)]
pub struct ConnectionConfig {
    /// Is this a client or server connection
    pub role: ConnectionRole,
    /// SRTP profiles to negotiate
    pub srtp_profiles: Vec<SrtpProfile>,
    /// Fingerprint algorithm to use
    pub fingerprint_algorithm: String,
    /// Certificate path if using custom certificate
    pub certificate_path: Option<String>,
    /// Private key path if using custom certificate
    pub private_key_path: Option<String>,
}

impl Default for ConnectionConfig {
    fn default() -> Self {
        Self {
            role: ConnectionRole::Server,
            srtp_profiles: vec![SrtpProfile::AesCm128HmacSha1_80, SrtpProfile::AesGcm128],
            fingerprint_algorithm: "sha-256".to_string(),
            certificate_path: None,
            private_key_path: None,
        }
    }
}

/// DTLS connection role
#[derive(Clone)]
pub enum ConnectionRole {
    /// Client role (initiates handshake)
    Client,
    /// Server role (responds to handshake)
    Server,
}

/// Server security configuration
#[derive(Debug, Clone)]
pub struct ServerSecurityConfig {
    /// Security mode to use
    pub security_mode: SecurityMode,
    /// DTLS fingerprint algorithm
    pub fingerprint_algorithm: String,
    /// Path to certificate file (PEM format)
    pub certificate_path: Option<String>,
    /// Path to private key file (PEM format)
    pub private_key_path: Option<String>,
    /// SRTP profiles supported (in order of preference)
    pub srtp_profiles: Vec<SrtpProfile>,
    /// Whether to require client certificate
    pub require_client_certificate: bool,
}

impl Default for ServerSecurityConfig {
    fn default() -> Self {
        Self {
            security_mode: SecurityMode::DtlsSrtp,
            fingerprint_algorithm: "sha-256".to_string(),
            certificate_path: None,
            private_key_path: None,
            srtp_profiles: vec![
                SrtpProfile::AesGcm128,
                SrtpProfile::AesCm128HmacSha1_80,
            ],
            require_client_certificate: false,
        }
    }
}

/// Client security context for a client connected to the server
///
/// This trait defines the interface for handling security with a specific client.
#[async_trait]
pub trait ClientSecurityContext: Send + Sync {
    /// Set the socket for the client security context
    async fn set_socket(&self, socket: SocketHandle) -> Result<(), SecurityError>;
    
    /// Get the client's fingerprint
    async fn get_remote_fingerprint(&self) -> Result<Option<String>, SecurityError>;
    
    /// Get the local fingerprint (server's fingerprint)
    async fn get_fingerprint(&self) -> Result<String, SecurityError>;
    
    /// Get the local fingerprint algorithm (server's algorithm)
    async fn get_fingerprint_algorithm(&self) -> Result<String, SecurityError>;
    
    /// Close the client security context
    async fn close(&self) -> Result<(), SecurityError>;
    
    /// Is the connection secure?
    fn is_secure(&self) -> bool;
    
    /// Get security information about this client
    fn get_security_info(&self) -> SecurityInfo;
    
    /// Wait for the DTLS handshake to complete
    async fn wait_for_handshake(&self) -> Result<(), SecurityError>;
    
    /// Verify if the handshake is complete
    async fn is_handshake_complete(&self) -> Result<bool, SecurityError>;
}

/// Server security context
///
/// This trait defines the interface for server-side security operations.
#[async_trait]
pub trait ServerSecurityContext: Send + Sync {
    /// Set the main socket for the server
    async fn set_socket(&self, socket: SocketHandle) -> Result<(), SecurityError>;
    
    /// Get the server's DTLS fingerprint
    async fn get_fingerprint(&self) -> Result<String, SecurityError>;
    
    /// Get the server's fingerprint algorithm
    async fn get_fingerprint_algorithm(&self) -> Result<String, SecurityError>;
    
    /// Start listening for incoming DTLS connections
    async fn start_listening(&self) -> Result<(), SecurityError>;
    
    /// Stop listening for incoming DTLS connections
    async fn stop_listening(&self) -> Result<(), SecurityError>;
    
    /// Create a security context for a new client
    async fn create_client_context(&self, addr: SocketAddr) -> Result<Arc<dyn ClientSecurityContext>, SecurityError>;
    
    /// Get all client security contexts
    async fn get_client_contexts(&self) -> Vec<Arc<dyn ClientSecurityContext>>;
    
    /// Remove a client security context
    async fn remove_client(&self, addr: SocketAddr) -> Result<(), SecurityError>;
    
    /// Register a callback for clients that complete security setup
    async fn on_client_secure(&self, callback: Box<dyn Fn(Arc<dyn ClientSecurityContext>) + Send + Sync>) -> Result<(), SecurityError>;
    
    /// Get the list of supported SRTP profiles
    async fn get_supported_srtp_profiles(&self) -> Vec<SrtpProfile>;
    
    /// Is the server using secure transport?
    fn is_secure(&self) -> bool;
    
    /// Get security information about the server
    fn get_security_info(&self) -> SecurityInfo;
}

/// Create a new server security context
pub async fn new(config: ServerSecurityConfig) -> Result<Arc<dyn ServerSecurityContext>, SecurityError> {
    DefaultServerSecurityContext::new(config).await
} 