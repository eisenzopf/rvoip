//! Server security API
//!
//! This module provides security-related interfaces for the server-side media transport.

use std::net::SocketAddr;
use std::sync::Arc;
use std::any::Any;
use async_trait::async_trait;
use tokio::net::UdpSocket;
use tokio::sync::Mutex;

use crate::api::common::error::SecurityError;
use crate::api::common::config::{SecurityInfo, SecurityMode, SrtpProfile};

// Import necessary modules as public
pub mod default;
pub mod core;
pub mod client;
pub mod dtls;
pub mod srtp;
pub mod util;

// Re-export DefaultServerSecurityContext
pub use default::DefaultServerSecurityContext;
pub use client::context::DefaultClientSecurityContext;

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
    /// Pre-shared SRTP key (for SRTP mode)
    pub srtp_key: Option<Vec<u8>>,
}

impl Default for ServerSecurityConfig {
    fn default() -> Self {
        Self {
            security_mode: SecurityMode::DtlsSrtp,
            fingerprint_algorithm: "sha-256".to_string(),
            certificate_path: None,
            private_key_path: None,
            srtp_profiles: vec![
                SrtpProfile::AesCm128HmacSha1_80,
                SrtpProfile::AesGcm128,
            ],
            require_client_certificate: false,
            srtp_key: None,
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
    
    /// Process a DTLS packet received from the client
    async fn process_dtls_packet(&self, data: &[u8]) -> Result<(), SecurityError>;
    
    /// Start a handshake with the remote
    async fn start_handshake_with_remote(&self, remote_addr: SocketAddr) -> Result<(), SecurityError>;
    
    /// Allow downcasting for internal implementation details
    fn as_any(&self) -> &dyn Any;
}

/// Server security context
///
/// This trait defines the interface for server-side security operations.
#[async_trait]
pub trait ServerSecurityContext: Send + Sync {
    /// Initialize the server security context
    async fn initialize(&self) -> Result<(), SecurityError>;
    
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
    
    /// Start automatic packet handler to process incoming DTLS packets
    /// This creates a background task that receives packets from the socket
    /// and automatically passes them to process_client_packet
    async fn start_packet_handler(&self) -> Result<(), SecurityError>;
    
    /// Capture the first packet from a client for proper handshake sequence
    async fn capture_initial_packet(&self) -> Result<Option<(Vec<u8>, SocketAddr)>, SecurityError>;
    
    /// Create a security context for a new client
    async fn create_client_context(&self, addr: SocketAddr) -> Result<Arc<dyn ClientSecurityContext + Send + Sync>, SecurityError>;
    
    /// Get all client security contexts
    async fn get_client_contexts(&self) -> Vec<Arc<dyn ClientSecurityContext + Send + Sync>>;
    
    /// Remove a client security context
    async fn remove_client(&self, addr: SocketAddr) -> Result<(), SecurityError>;
    
    /// Register a callback for clients that complete security setup
    async fn on_client_secure(&self, callback: Box<dyn Fn(Arc<dyn ClientSecurityContext + Send + Sync>) + Send + Sync>) -> Result<(), SecurityError>;
    
    /// Get the list of supported SRTP profiles
    async fn get_supported_srtp_profiles(&self) -> Vec<SrtpProfile>;
    
    /// Is the server using secure transport?
    fn is_secure(&self) -> bool;
    
    /// Get security information about the server
    fn get_security_info(&self) -> SecurityInfo;
    
    /// Process a packet from a specific client for DTLS handshake
    async fn process_client_packet(&self, addr: SocketAddr, data: &[u8]) -> Result<(), SecurityError>;
    
    /// Check if the server is fully initialized and ready to process handshake messages
    /// This verifies that all prerequisites (socket, etc.) are set
    async fn is_ready(&self) -> Result<bool, SecurityError>;
    
    /// Get the security configuration
    fn get_config(&self) -> &ServerSecurityConfig;
}

/// Create a new server security context
pub async fn new(config: ServerSecurityConfig) -> Result<Arc<dyn ServerSecurityContext + Send + Sync>, SecurityError> {
    match config.security_mode {
        SecurityMode::Srtp => {
            // Use SRTP-only context for pre-shared keys (no DTLS handshake)
            let srtp_ctx = srtp::SrtpServerSecurityContext::new(config).await?;
            Ok(srtp_ctx as Arc<dyn ServerSecurityContext + Send + Sync>)
        },
        SecurityMode::DtlsSrtp => {
            // Use DTLS-SRTP context for handshake-based keys
            let dtls_ctx = DefaultServerSecurityContext::new(config).await?;
            Ok(dtls_ctx as Arc<dyn ServerSecurityContext + Send + Sync>)
        },
        SecurityMode::SdesSrtp |
        SecurityMode::MikeySrtp |
        SecurityMode::ZrtpSrtp => {
            // For now, treat these as DTLS-based (they would need specific implementations)
            let dtls_ctx = DefaultServerSecurityContext::new(config).await?;
            Ok(dtls_ctx as Arc<dyn ServerSecurityContext + Send + Sync>)
        },
        SecurityMode::None => {
            // For None, we could return a no-op security context, but for now use DTLS as fallback
            let dtls_ctx = DefaultServerSecurityContext::new(config).await?;
            Ok(dtls_ctx as Arc<dyn ServerSecurityContext + Send + Sync>)
        }
    }
} 