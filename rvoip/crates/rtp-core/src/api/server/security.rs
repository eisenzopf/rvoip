//! Server security API
//!
//! This module provides the server-specific security interface for media transport.

use std::sync::Arc;
use std::net::SocketAddr;
use async_trait::async_trait;

use crate::api::common::error::SecurityError;
use crate::api::common::config::{SecurityInfo, SecurityMode, SrtpProfile};

// Implementation module
pub mod security_impl;

/// Security configuration specifically for servers
#[derive(Debug, Clone)]
pub struct ServerSecurityConfig {
    /// Security mode to use
    pub mode: SecurityMode,
    
    /// SRTP protection profiles in order of preference
    pub srtp_profiles: Vec<SrtpProfile>,
    
    /// Whether to require secure transport
    pub require_secure: bool,
    
    /// Pre-shared SRTP key material (only used in SrtpWithPsk mode)
    pub psk_material: Option<Vec<u8>>,
}

impl Default for ServerSecurityConfig {
    fn default() -> Self {
        Self {
            mode: SecurityMode::DtlsSrtp,
            srtp_profiles: vec![
                SrtpProfile::AesGcm128,
                SrtpProfile::AesCm128HmacSha1_80,
            ],
            require_secure: true,
            psk_material: None,
        }
    }
}

/// Client security context for a server
#[derive(Debug, Clone)]
pub struct ClientSecurityContext {
    /// Remote address of the client
    pub address: SocketAddr,
    
    /// Whether the connection is secure
    pub is_secure: bool,
    
    /// Client's DTLS fingerprint, if available
    pub client_fingerprint: Option<String>,
    
    /// Client's fingerprint algorithm, if available
    pub client_fingerprint_algorithm: Option<String>,
    
    /// Negotiated SRTP profile, if any
    pub negotiated_profile: Option<SrtpProfile>,
}

/// Server-specific secure context operations
#[async_trait]
pub trait ServerSecurityContext: Send + Sync {
    /// Get security information for SDP
    fn get_security_info(&self) -> SecurityInfo;
    
    /// Check if the context is secure (at least one successful handshake)
    fn is_secure(&self) -> bool;
    
    /// Start listening for DTLS connections
    async fn start_listening(&self) -> Result<(), SecurityError>;
    
    /// Stop listening for DTLS connections
    async fn stop_listening(&self) -> Result<(), SecurityError>;
    
    /// Get a list of connected client security contexts
    async fn get_client_contexts(&self) -> Vec<ClientSecurityContext>;
    
    /// Remove a client by address
    async fn remove_client(&self, addr: SocketAddr) -> Result<(), SecurityError>;
    
    /// Set the transport socket for DTLS
    async fn set_transport_socket(&self, socket: std::sync::Arc<tokio::net::UdpSocket>) -> Result<(), SecurityError>;
    
    /// Register a callback for client handshake completion
    fn on_client_secure(&self, callback: Box<dyn Fn(ClientSecurityContext) + Send + Sync>) -> Result<(), SecurityError>;
}

/// Factory for creating ServerSecurityContext instances
pub struct ServerSecurityFactory;

impl ServerSecurityFactory {
    /// Create a new ServerSecurityContext
    pub async fn create_context(
        config: ServerSecurityConfig,
    ) -> Result<Arc<dyn ServerSecurityContext>, SecurityError> {
        // Delegate to the implementation module
        let context = security_impl::DefaultServerSecurityContext::new(config).await?;
        Ok(context as Arc<dyn ServerSecurityContext>)
    }
} 