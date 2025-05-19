//! Client security API
//!
//! This module provides the client-specific security interface for media transport.

use std::sync::Arc;
use async_trait::async_trait;

use crate::api::common::error::SecurityError;
use crate::api::common::config::{SecurityInfo, SecurityMode, SrtpProfile};

// Implementation module
pub mod security_impl;

/// Security configuration specifically for clients
#[derive(Debug, Clone)]
pub struct ClientSecurityConfig {
    /// Security mode to use
    pub mode: SecurityMode,
    
    /// SRTP protection profiles in order of preference
    pub srtp_profiles: Vec<SrtpProfile>,
    
    /// Whether to require secure transport
    pub require_secure: bool,
    
    /// Pre-shared SRTP key material (only used in SrtpWithPsk mode)
    pub psk_material: Option<Vec<u8>>,
}

impl Default for ClientSecurityConfig {
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

/// Client-specific secure context operations
#[async_trait]
pub trait ClientSecurityContext: Send + Sync {
    /// Get security information for SDP
    fn get_security_info(&self) -> SecurityInfo;
    
    /// Check if the context is secure (handshake completed)
    fn is_secure(&self) -> bool;
    
    /// Set remote fingerprint from SDP
    async fn set_remote_fingerprint(&mut self, fingerprint: &str, algorithm: &str) 
        -> Result<(), SecurityError>;
    
    /// Set the remote address for DTLS communications
    async fn set_remote_address(&self, addr: std::net::SocketAddr) -> Result<(), SecurityError>;
    
    /// Start the DTLS handshake as client
    ///
    /// This initiates the DTLS handshake by sending a ClientHello message.
    async fn start_client_handshake(&self) -> Result<(), SecurityError>;
    
    /// Wait for the DTLS handshake to complete
    async fn wait_handshake(&self) -> Result<(), SecurityError>;
    
    /// Set the transport socket for DTLS
    async fn set_transport_socket(&self, socket: std::sync::Arc<tokio::net::UdpSocket>) -> Result<(), SecurityError>;
}

/// Factory for creating ClientSecurityContext instances
pub struct ClientSecurityFactory;

impl ClientSecurityFactory {
    /// Create a new ClientSecurityContext
    pub async fn create_context(
        config: ClientSecurityConfig,
    ) -> Result<Arc<dyn ClientSecurityContext>, SecurityError> {
        // Delegate to the implementation module
        let context = security_impl::DefaultClientSecurityContext::new(config).await?;
        Ok(context as Arc<dyn ClientSecurityContext>)
    }
} 