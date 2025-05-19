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
pub use transport::client_transport_impl::DefaultMediaTransportClient;
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