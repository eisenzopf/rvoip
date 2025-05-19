//! Client transport API
//!
//! This module provides the client-specific transport interface for media transport.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use async_trait::async_trait;

use crate::api::common::frame::MediaFrame;
use crate::api::common::error::MediaTransportError;
use crate::api::common::events::MediaEventCallback;
use crate::api::client::config::ClientConfig;
use crate::api::security::SecurityInfo;
use crate::api::stats::MediaStats;

// Implementation module
pub mod transport_impl;

/// Client implementation of the media transport interface
#[async_trait]
pub trait MediaTransportClient: Send + Sync {
    /// Connect to the remote peer
    ///
    /// This starts the client media transport, establishing connections with the
    /// remote peer specified in the configuration.
    async fn connect(&self) -> Result<(), MediaTransportError>;
    
    /// Disconnect from the remote peer
    ///
    /// This ends the client media transport session, closing connections and
    /// releasing resources.
    async fn disconnect(&self) -> Result<(), MediaTransportError>;
    
    /// Send a media frame to the remote peer
    async fn send_frame(&self, frame: MediaFrame) -> Result<(), MediaTransportError>;
    
    /// Receive a media frame from the remote peer
    ///
    /// This method returns the next available media frame, or None if no frame
    /// is available within the specified timeout.
    async fn receive_frame(&self, timeout: Duration) -> Result<Option<MediaFrame>, MediaTransportError>;
    
    /// Update the remote address
    ///
    /// This can be used to update the remote address after ICE negotiation.
    async fn set_remote_address(&self, addr: SocketAddr) -> Result<(), MediaTransportError>;
    
    /// Register a callback for transport events
    fn on_event(&self, callback: MediaEventCallback) -> Result<(), MediaTransportError>;
    
    /// Get current transport statistics
    async fn get_stats(&self) -> Result<MediaStats, MediaTransportError>;
    
    /// Get security information for SDP exchange
    ///
    /// This returns security information like DTLS fingerprint that can be
    /// included in SDP for secure media negotiation.
    async fn get_security_info(&self) -> Result<SecurityInfo, MediaTransportError>;
    
    /// Set the remote fingerprint for DTLS-SRTP
    ///
    /// This should be called with the fingerprint received from the remote peer's SDP
    /// before connecting when using DTLS-SRTP.
    async fn set_remote_fingerprint(&self, fingerprint: &str, algorithm: &str) -> Result<(), MediaTransportError>;
}

/// Factory for creating MediaTransportClient instances
pub struct ClientFactory;

impl ClientFactory {
    /// Create a new MediaTransportClient
    pub async fn create_client(
        config: ClientConfig,
    ) -> Result<Arc<dyn MediaTransportClient>, MediaTransportError> {
        // Delegate to the implementation module
        let client = transport_impl::DefaultMediaTransportClient::new(config).await?;
        Ok(client as Arc<dyn MediaTransportClient>)
    }
} 