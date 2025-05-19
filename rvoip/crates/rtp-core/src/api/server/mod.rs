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
pub use transport::server_transport_impl::DefaultMediaTransportServer;
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