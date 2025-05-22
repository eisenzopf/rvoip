//! Client API for media transport
//!
//! This module provides client-side API components for media transport.

pub mod transport;
pub mod security;
pub mod config;

// Re-export public API
pub use transport::MediaTransportClient;
pub use security::{ClientSecurityContext, ClientSecurityConfig};
pub use config::{ClientConfig, ClientConfigBuilder};

// Re-export implementation files
pub use transport::default::DefaultMediaTransportClient;
pub use security::DefaultClientSecurityContext;

// Import errors
use crate::api::common::error::MediaTransportError;

use std::sync::Arc;

/// Factory for creating media transport clients
pub struct ClientFactory;

impl ClientFactory {
    /// Create a new media transport client
    pub async fn create_client(config: ClientConfig) -> Result<DefaultMediaTransportClient, MediaTransportError> {
        // Create client transport
        let client = DefaultMediaTransportClient::new(config).await
            .map_err(|e| MediaTransportError::InitializationError(format!("Failed to create client: {}", e)))?;
        
        Ok(client)
    }
    
    /// Create a client for WebRTC
    pub async fn create_webrtc_client(
        remote_addr: std::net::SocketAddr
    ) -> Result<DefaultMediaTransportClient, MediaTransportError> {
        // Create WebRTC-optimized config
        let config = ClientConfigBuilder::webrtc()
            .remote_address(remote_addr)
            .build();
            
        Self::create_client(config).await
    }
    
    /// Create a client for SIP
    pub async fn create_sip_client(
        remote_addr: std::net::SocketAddr
    ) -> Result<DefaultMediaTransportClient, MediaTransportError> {
        // Create SIP-optimized config
        let config = ClientConfigBuilder::sip()
            .remote_address(remote_addr)
            .build();
            
        Self::create_client(config).await
    }
}

// Update the ClientConfigBuilder to add a method for the unified security config
impl ClientConfigBuilder {
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
                // Convert to security config format expected by client
                let client_security_config = crate::api::client::security::ClientSecurityConfig {
                    security_mode: crate::api::common::config::SecurityMode::Srtp,
                    fingerprint_algorithm: security_config.fingerprint_algorithm,
                    remote_fingerprint: security_config.remote_fingerprint.clone(),
                    remote_fingerprint_algorithm: security_config.remote_fingerprint_algorithm.clone(),
                    validate_fingerprint: false, // Not used for SRTP mode
                    srtp_profiles: security_config.srtp_profiles,
                    certificate_path: None, // Not used for SRTP mode
                    private_key_path: None, // Not used for SRTP mode
                };
                
                self = self.security_config(client_security_config);
                
                // If a key was provided, set it up for SRTP
                if let Some(key) = security_config.srtp_key {
                    // Here you would set up the pre-shared key
                    // This might require additional implementation in your SRTP code
                }
            },
            crate::api::common::config::SecurityMode::DtlsSrtp => {
                // DTLS-SRTP mode
                // Convert to security config format expected by client
                let client_security_config = crate::api::client::security::ClientSecurityConfig {
                    security_mode: crate::api::common::config::SecurityMode::DtlsSrtp,
                    fingerprint_algorithm: security_config.fingerprint_algorithm,
                    remote_fingerprint: security_config.remote_fingerprint.clone(),
                    remote_fingerprint_algorithm: security_config.remote_fingerprint_algorithm.clone(),
                    validate_fingerprint: security_config.remote_fingerprint.is_some(),
                    srtp_profiles: security_config.srtp_profiles,
                    certificate_path: security_config.certificate_path,
                    private_key_path: security_config.private_key_path,
                };
                
                self = self.security_config(client_security_config);
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