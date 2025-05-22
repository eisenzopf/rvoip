//! Server API for media transport
//!
//! This module provides server-side API components for media transport.

pub mod transport;
pub mod security;
pub mod config;

// Re-export public API
pub use transport::{MediaTransportServer, ClientInfo};
pub use security::{ServerSecurityContext, ServerSecurityConfig};
pub use config::{ServerConfig, ServerConfigBuilder};

// Re-export implementation files
pub use transport::DefaultMediaTransportServer;
pub use security::server_security_impl::DefaultServerSecurityContext;

// Import errors
use crate::api::common::error::MediaTransportError;

use std::sync::Arc;

/// Factory for creating media transport servers
pub struct ServerFactory;

impl ServerFactory {
    /// Create a new media transport server
    pub async fn create_server(config: ServerConfig) -> Result<DefaultMediaTransportServer, MediaTransportError> {
        // Create the server
        let server = DefaultMediaTransportServer::new(config).await?;
        Ok(server)
    }
    
    /// Create a server for WebRTC
    pub async fn create_webrtc_server(
        local_addr: std::net::SocketAddr
    ) -> Result<DefaultMediaTransportServer, MediaTransportError> {
        // Create WebRTC-optimized config
        let config = ServerConfigBuilder::webrtc()
            .local_address(local_addr)
            .build()?;
            
        Self::create_server(config).await
    }
    
    /// Create a server for SIP
    pub async fn create_sip_server(
        local_addr: std::net::SocketAddr
    ) -> Result<DefaultMediaTransportServer, MediaTransportError> {
        // Create SIP-optimized config
        let config = ServerConfigBuilder::sip()
            .local_address(local_addr)
            .build()?;
            
        Self::create_server(config).await
    }
    
    /// Create a high-capacity server
    pub async fn create_high_capacity_server(
        local_addr: std::net::SocketAddr,
        max_clients: usize
    ) -> Result<DefaultMediaTransportServer, MediaTransportError> {
        // Create high-capacity config
        let config = ServerConfigBuilder::new()
            .local_address(local_addr)
            .max_clients(max_clients)
            .build()?;
            
        Self::create_server(config).await
    }
}

// Update the ServerConfigBuilder to add a method for the unified security config
impl ServerConfigBuilder {
    /// Set the security configuration using the unified SecurityConfig
    /// This provides an easier way to configure security with predefined profiles
    pub fn with_security(mut self, security_config: crate::api::common::config::SecurityConfig) -> Self {
        match security_config.mode {
            crate::api::common::config::SecurityMode::None => {
                // No security - use plain RTP
                // Don't set any security config
            },
            crate::api::common::config::SecurityMode::Srtp => {
                // Basic SRTP with pre-shared key
                // Convert to security config format expected by server
                let server_security_config = crate::api::server::security::ServerSecurityConfig {
                    security_mode: crate::api::common::config::SecurityMode::Srtp,
                    fingerprint_algorithm: security_config.fingerprint_algorithm,
                    srtp_profiles: security_config.srtp_profiles,
                    certificate_path: None, // Not used for SRTP mode 
                    private_key_path: None, // Not used for SRTP mode
                    require_client_certificate: false, // Not used for SRTP mode
                };
                
                self = self.security_config(server_security_config);
                
                // If a key was provided, set it up for SRTP
                if let Some(key) = security_config.srtp_key {
                    // Here you would set up the pre-shared key
                    // This might require additional implementation in your SRTP code
                }
            },
            crate::api::common::config::SecurityMode::DtlsSrtp => {
                // DTLS-SRTP mode
                // Convert to security config format expected by server
                let server_security_config = crate::api::server::security::ServerSecurityConfig {
                    security_mode: crate::api::common::config::SecurityMode::DtlsSrtp,
                    fingerprint_algorithm: security_config.fingerprint_algorithm,
                    srtp_profiles: security_config.srtp_profiles,
                    certificate_path: security_config.certificate_path,
                    private_key_path: security_config.private_key_path,
                    require_client_certificate: security_config.require_client_certificate,
                };
                
                self = self.security_config(server_security_config);
            }
        }
        
        self
    }
    
    /// Set up WebRTC-compatible security (DTLS-SRTP with self-signed certs)
    pub fn with_webrtc_security(self) -> Self {
        let security_config = crate::api::common::config::SecurityConfig::webrtc_compatible();
        self.with_security(security_config)
    }
    
    /// Set up SRTP with a pre-shared key
    pub fn with_srtp_key(self, key: Vec<u8>) -> Self {
        let security_config = crate::api::common::config::SecurityConfig::srtp_with_key(key);
        self.with_security(security_config)
    }
    
    /// Set up plain RTP (no security)
    pub fn with_no_security(self) -> Self {
        let security_config = crate::api::common::config::SecurityConfig::unsecured();
        self.with_security(security_config)
    }
    
    /// Set up DTLS-SRTP with provided certificate files
    pub fn with_dtls_certificate(self, cert_path: String, key_path: String) -> Self {
        let security_config = crate::api::common::config::SecurityConfig::dtls_with_certificate(cert_path, key_path);
        self.with_security(security_config)
    }
} 