//! Server transport API
//!
//! This module provides the server-specific transport interface for media transport.

use std::net::SocketAddr;
use std::sync::Arc;
use async_trait::async_trait;
use std::time::Duration;

use crate::api::common::frame::MediaFrame;
use crate::api::common::error::MediaTransportError;
use crate::api::common::events::MediaEventCallback;
use crate::api::common::config::SecurityInfo;
use crate::api::common::stats::MediaStats;
use crate::api::server::config::ServerConfig;
use crate::api::client::transport::RtcpStats;

pub mod server_transport_impl;

// Re-export the implementation
pub use server_transport_impl::DefaultMediaTransportServer;

/// Client information
#[derive(Debug, Clone)]
pub struct ClientInfo {
    /// Client identifier
    pub id: String,
    /// Client address
    pub address: SocketAddr,
    /// Is the connection secure
    pub secure: bool,
    /// Security information (if secure)
    pub security_info: Option<SecurityInfo>,
    /// Is the client connected
    pub connected: bool,
}

/// Server implementation of the media transport interface
#[async_trait]
pub trait MediaTransportServer: Send + Sync {
    /// Start the server
    ///
    /// This binds to the configured address and starts listening for
    /// incoming client connections.
    async fn start(&self) -> Result<(), MediaTransportError>;
    
    /// Stop the server
    ///
    /// This stops listening for new connections and disconnects all
    /// existing clients.
    async fn stop(&self) -> Result<(), MediaTransportError>;
    
    /// Get the local address currently bound to
    /// 
    /// This returns the actual bound address of the transport, which may be different
    /// from the configured address if dynamic port allocation is used. When using
    /// dynamic port allocation, this method should be called after start() to
    /// get the allocated port.
    /// 
    /// This information is needed for SDP exchange in signaling protocols.
    async fn get_local_address(&self) -> Result<SocketAddr, MediaTransportError>;
    
    /// Send a media frame to a specific client
    ///
    /// If the client is not connected, this will return an error.
    async fn send_frame_to(&self, client_id: &str, frame: MediaFrame) -> Result<(), MediaTransportError>;
    
    /// Broadcast a media frame to all connected clients
    async fn broadcast_frame(&self, frame: MediaFrame) -> Result<(), MediaTransportError>;
    
    /// Receive a media frame from any client
    ///
    /// This returns the client ID and the frame received.
    async fn receive_frame(&self) -> Result<(String, MediaFrame), MediaTransportError>;
    
    /// Get a list of connected clients
    async fn get_clients(&self) -> Result<Vec<ClientInfo>, MediaTransportError>;
    
    /// Disconnect a specific client
    async fn disconnect_client(&self, client_id: &str) -> Result<(), MediaTransportError>;
    
    /// Register a callback for transport events
    async fn on_event(&self, callback: MediaEventCallback) -> Result<(), MediaTransportError>;
    
    /// Register a callback for client connection events
    async fn on_client_connected(&self, callback: Box<dyn Fn(ClientInfo) + Send + Sync>) -> Result<(), MediaTransportError>;
    
    /// Register a callback for client disconnection events
    async fn on_client_disconnected(&self, callback: Box<dyn Fn(ClientInfo) + Send + Sync>) -> Result<(), MediaTransportError>;
    
    /// Get statistics for all clients
    async fn get_stats(&self) -> Result<MediaStats, MediaTransportError>;
    
    /// Get statistics for a specific client
    async fn get_client_stats(&self, client_id: &str) -> Result<MediaStats, MediaTransportError>;
    
    /// Get security information for SDP exchange
    async fn get_security_info(&self) -> Result<SecurityInfo, MediaTransportError>;
    
    /// Send an RTCP Receiver Report to all clients
    ///
    /// This sends a Receiver Report RTCP packet to all connected clients. This can be
    /// useful to force an immediate quality report instead of waiting for the
    /// automatic interval-based reports.
    async fn send_rtcp_receiver_report(&self) -> Result<(), MediaTransportError>;
    
    /// Send an RTCP Sender Report to all clients
    ///
    /// This sends a Sender Report RTCP packet to all connected clients. This can be
    /// useful to force an immediate quality report instead of waiting for the
    /// automatic interval-based reports.
    async fn send_rtcp_sender_report(&self) -> Result<(), MediaTransportError>;
    
    /// Send an RTCP Receiver Report to a specific client
    ///
    /// This sends a Receiver Report RTCP packet to the specified client. This can be
    /// useful to force an immediate quality report for a specific client.
    async fn send_rtcp_receiver_report_to_client(&self, client_id: &str) -> Result<(), MediaTransportError>;
    
    /// Send an RTCP Sender Report to a specific client
    ///
    /// This sends a Sender Report RTCP packet to the specified client. This can be
    /// useful to force an immediate quality report for a specific client.
    async fn send_rtcp_sender_report_to_client(&self, client_id: &str) -> Result<(), MediaTransportError>;
    
    /// Get detailed RTCP statistics for all clients
    ///
    /// This returns detailed quality metrics gathered from RTCP reports
    /// including jitter, packet loss, and round-trip time, aggregated across all clients.
    async fn get_rtcp_stats(&self) -> Result<RtcpStats, MediaTransportError>;
    
    /// Get detailed RTCP statistics for a specific client
    ///
    /// This returns detailed quality metrics gathered from RTCP reports
    /// including jitter, packet loss, and round-trip time for a specific client.
    async fn get_client_rtcp_stats(&self, client_id: &str) -> Result<RtcpStats, MediaTransportError>;
    
    /// Set the RTCP report interval
    ///
    /// This sets how frequently RTCP reports are sent. The default is usually
    /// 5% of the session bandwidth, but this can be adjusted for more or less
    /// frequent reporting.
    async fn set_rtcp_interval(&self, interval: Duration) -> Result<(), MediaTransportError>;
} 