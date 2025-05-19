//! Server transport API
//!
//! This module provides the server-specific transport interface for media transport.

use std::net::SocketAddr;
use std::sync::Arc;
use async_trait::async_trait;

use crate::api::common::frame::MediaFrame;
use crate::api::common::error::MediaTransportError;
use crate::api::common::events::MediaEventCallback;
use crate::api::server::config::ServerConfig;
use crate::api::security::SecurityInfo;
use crate::api::stats::MediaStats;

// Implementation module
pub mod transport_impl;

/// Client information on a server
#[derive(Debug, Clone)]
pub struct ClientInfo {
    /// Client identifier
    pub id: String,
    
    /// Remote address of the client
    pub address: SocketAddr,
    
    /// Whether the client has a secure connection
    pub secure: bool,
    
    /// Security information for this client, if available
    pub security_info: Option<SecurityInfo>,
    
    /// Connection status
    pub connected: bool,
}

/// Server implementation of the media transport interface
#[async_trait]
pub trait MediaTransportServer: Send + Sync {
    /// Start the server
    ///
    /// This starts listening for incoming connections.
    async fn start(&self) -> Result<(), MediaTransportError>;
    
    /// Stop the server
    ///
    /// This stops listening and disconnects all clients.
    async fn stop(&self) -> Result<(), MediaTransportError>;
    
    /// Send a media frame to a specific client
    async fn send_frame_to(&self, client_id: &str, frame: MediaFrame) -> Result<(), MediaTransportError>;
    
    /// Send a media frame to all connected clients
    async fn broadcast_frame(&self, frame: MediaFrame) -> Result<(), MediaTransportError>;
    
    /// Receive a media frame from any client
    ///
    /// Returns the client ID along with the frame.
    async fn receive_frame(&self) -> Result<(String, MediaFrame), MediaTransportError>;
    
    /// Get the list of connected clients
    async fn get_clients(&self) -> Result<Vec<ClientInfo>, MediaTransportError>;
    
    /// Disconnect a specific client
    async fn disconnect_client(&self, client_id: &str) -> Result<(), MediaTransportError>;
    
    /// Register a callback for server events
    fn on_event(&self, callback: MediaEventCallback) -> Result<(), MediaTransportError>;
    
    /// Register a callback for client connection events
    fn on_client_connected(&self, callback: Box<dyn Fn(ClientInfo) + Send + Sync>) -> Result<(), MediaTransportError>;
    
    /// Register a callback for client disconnection events
    fn on_client_disconnected(&self, callback: Box<dyn Fn(ClientInfo) + Send + Sync>) -> Result<(), MediaTransportError>;
    
    /// Get overall server statistics
    async fn get_stats(&self) -> Result<MediaStats, MediaTransportError>;
    
    /// Get statistics for a specific client
    async fn get_client_stats(&self, client_id: &str) -> Result<MediaStats, MediaTransportError>;
    
    /// Get server security information (including fingerprint for SDP)
    async fn get_security_info(&self) -> Result<SecurityInfo, MediaTransportError>;
}

/// Factory for creating MediaTransportServer instances
pub struct ServerFactory;

impl ServerFactory {
    /// Create a new MediaTransportServer
    pub async fn create_server(
        config: ServerConfig,
    ) -> Result<Arc<dyn MediaTransportServer>, MediaTransportError> {
        // Delegate to the implementation module
        let server = transport_impl::DefaultMediaTransportServer::new(config).await?;
        Ok(server as Arc<dyn MediaTransportServer>)
    }
} 