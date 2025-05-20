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
use crate::api::common::config::SecurityInfo;
use crate::api::common::stats::MediaStats;

pub mod client_transport_impl;

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
    /// This stops the client media transport, closing all connections and
    /// releasing resources.
    async fn disconnect(&self) -> Result<(), MediaTransportError>;
    
    /// Get the local address currently bound to
    /// 
    /// This returns the actual bound address of the transport, which may be different
    /// from the configured address if dynamic port allocation is used. When using
    /// dynamic port allocation, this method should be called after connect() to
    /// get the allocated port.
    /// 
    /// This information is needed for SDP exchange in signaling protocols.
    async fn get_local_address(&self) -> Result<SocketAddr, MediaTransportError>;
    
    /// Send a media frame to the server
    ///
    /// This sends a media frame to the remote peer. The frame will be encrypted
    /// if security is enabled.
    async fn send_frame(&self, frame: MediaFrame) -> Result<(), MediaTransportError>;
    
    /// Receive a media frame from the server
    ///
    /// This receives a media frame from the remote peer. The frame will be decrypted
    /// if security is enabled. If no frame is available within the timeout, returns Ok(None).
    async fn receive_frame(&self, timeout: Duration) -> Result<Option<MediaFrame>, MediaTransportError>;
    
    /// Check if the client is connected
    ///
    /// This returns true if the client is connected to the remote peer.
    async fn is_connected(&self) -> Result<bool, MediaTransportError>;
    
    /// Register a callback for connection events
    ///
    /// The callback will be invoked when the connection state changes.
    async fn on_connect(&self, callback: Box<dyn Fn() + Send + Sync>) -> Result<(), MediaTransportError>;
    
    /// Register a callback for disconnection events
    ///
    /// The callback will be invoked when the client disconnects from the remote peer.
    async fn on_disconnect(&self, callback: Box<dyn Fn() + Send + Sync>) -> Result<(), MediaTransportError>;
    
    /// Register a callback for generic transport events
    ///
    /// The callback will be invoked for various transport-related events.
    async fn on_event(&self, callback: MediaEventCallback) -> Result<(), MediaTransportError>;
    
    /// Get connection statistics
    ///
    /// This returns statistics about the media transport connection.
    async fn get_stats(&self) -> Result<MediaStats, MediaTransportError>;
    
    /// Get security information for SDP exchange
    ///
    /// This returns information needed for the secure transport setup.
    async fn get_security_info(&self) -> Result<SecurityInfo, MediaTransportError>;
    
    /// Check if secure transport is being used
    ///
    /// This returns true if secure transport (DTLS/SRTP) is enabled.
    fn is_secure(&self) -> bool;
    
    /// Set the jitter buffer size
    ///
    /// This sets the size of the jitter buffer in milliseconds.
    async fn set_jitter_buffer_size(&self, size_ms: Duration) -> Result<(), MediaTransportError>;
}

// Re-export the implementation
pub use client_transport_impl::DefaultMediaTransportClient; 