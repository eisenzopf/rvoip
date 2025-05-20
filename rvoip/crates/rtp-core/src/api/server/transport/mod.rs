//! Server transport API
//!
//! This module provides the server-specific transport interface for media transport.

use std::net::SocketAddr;
use std::sync::Arc;
use async_trait::async_trait;
use std::time::Duration;
use std::collections::HashMap;

use crate::api::common::frame::MediaFrame;
use crate::api::common::error::MediaTransportError;
use crate::api::common::events::MediaEventCallback;
use crate::api::common::config::SecurityInfo;
use crate::api::common::stats::MediaStats;
use crate::api::common::extension::ExtensionFormat;
use crate::api::server::config::ServerConfig;
use crate::api::client::transport::RtcpStats;
use crate::api::client::transport::VoipMetrics;
use crate::{CsrcMapping, RtpSsrc, RtpCsrc};

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

/// RTP Header Extension Data
#[derive(Debug, Clone)]
pub struct HeaderExtension {
    /// The ID of the extension (1-14 for one-byte header, 1-255 for two-byte header)
    pub id: u8,
    
    /// The URI that identifies this extension type
    pub uri: String,
    
    /// The data of the extension
    pub data: Vec<u8>,
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
    
    /// Send an RTCP Application-Defined (APP) packet to all clients
    ///
    /// This sends an RTCP APP packet with the specified name and application data
    /// to all connected clients. APP packets are used for application-specific
    /// purposes and allow custom data to be exchanged between endpoints.
    ///
    /// - `name`: A four-character ASCII name to identify the application
    /// - `data`: The application-specific data to send
    async fn send_rtcp_app(&self, name: &str, data: Vec<u8>) -> Result<(), MediaTransportError>;
    
    /// Send an RTCP Application-Defined (APP) packet to a specific client
    ///
    /// This sends an RTCP APP packet with the specified name and application data
    /// to the specified client. APP packets are used for application-specific
    /// purposes and allow custom data to be exchanged between endpoints.
    ///
    /// - `client_id`: The ID of the client to send the packet to
    /// - `name`: A four-character ASCII name to identify the application
    /// - `data`: The application-specific data to send
    async fn send_rtcp_app_to_client(&self, client_id: &str, name: &str, data: Vec<u8>) -> Result<(), MediaTransportError>;
    
    /// Send an RTCP Goodbye (BYE) packet to all clients
    ///
    /// This sends an RTCP BYE packet with an optional reason for leaving.
    /// BYE packets are used to indicate that a source is no longer active.
    ///
    /// - `reason`: An optional reason string for leaving
    async fn send_rtcp_bye(&self, reason: Option<String>) -> Result<(), MediaTransportError>;
    
    /// Send an RTCP Goodbye (BYE) packet to a specific client
    ///
    /// This sends an RTCP BYE packet with an optional reason for leaving
    /// to the specified client. BYE packets are used to indicate that a
    /// source is no longer active.
    ///
    /// - `client_id`: The ID of the client to send the packet to
    /// - `reason`: An optional reason string for leaving
    async fn send_rtcp_bye_to_client(&self, client_id: &str, reason: Option<String>) -> Result<(), MediaTransportError>;
    
    /// Send an RTCP Extended Report (XR) packet with VoIP metrics to all clients
    ///
    /// This sends an RTCP XR packet with VoIP metrics for the specified SSRC
    /// to all connected clients. XR packets are used to report extended
    /// statistics beyond what is available in standard Sender/Receiver Reports.
    ///
    /// - `metrics`: The VoIP metrics to include in the XR packet
    async fn send_rtcp_xr_voip_metrics(&self, metrics: VoipMetrics) -> Result<(), MediaTransportError>;
    
    /// Send an RTCP Extended Report (XR) packet with VoIP metrics to a specific client
    ///
    /// This sends an RTCP XR packet with VoIP metrics for the specified SSRC
    /// to the specified client. XR packets are used to report extended
    /// statistics beyond what is available in standard Sender/Receiver Reports.
    ///
    /// - `client_id`: The ID of the client to send the packet to
    /// - `metrics`: The VoIP metrics to include in the XR packet
    async fn send_rtcp_xr_voip_metrics_to_client(&self, client_id: &str, metrics: VoipMetrics) -> Result<(), MediaTransportError>;
    
    // CSRC Management API Methods
    
    /// Check if CSRC management is enabled
    ///
    /// Returns true if CSRC management is enabled for this server.
    async fn is_csrc_management_enabled(&self) -> Result<bool, MediaTransportError>;
    
    /// Enable CSRC management
    ///
    /// This enables the CSRC management feature if it was not enabled
    /// in the configuration. Returns true if successfully enabled.
    async fn enable_csrc_management(&self) -> Result<bool, MediaTransportError>;
    
    /// Add a CSRC mapping for contributing sources
    ///
    /// Maps an original SSRC to a CSRC value with optional metadata.
    /// This is used in conferencing scenarios where multiple sources
    /// contribute to a single mixed stream.
    ///
    /// - `mapping`: The CSRC mapping to add
    async fn add_csrc_mapping(&self, mapping: CsrcMapping) -> Result<(), MediaTransportError>;
    
    /// Add a simple SSRC to CSRC mapping
    ///
    /// Simplified version that just maps an SSRC to a CSRC without metadata.
    ///
    /// - `original_ssrc`: The original SSRC to map
    /// - `csrc`: The CSRC value to map to
    async fn add_simple_csrc_mapping(&self, original_ssrc: RtpSsrc, csrc: RtpCsrc) 
        -> Result<(), MediaTransportError>;
    
    /// Remove a CSRC mapping by SSRC
    ///
    /// Removes a mapping for the specified SSRC.
    ///
    /// - `original_ssrc`: The original SSRC to remove mapping for
    async fn remove_csrc_mapping_by_ssrc(&self, original_ssrc: RtpSsrc) 
        -> Result<Option<CsrcMapping>, MediaTransportError>;
    
    /// Get a CSRC mapping by SSRC
    ///
    /// Returns the mapping for the specified SSRC if it exists.
    ///
    /// - `original_ssrc`: The original SSRC to get mapping for
    async fn get_csrc_mapping_by_ssrc(&self, original_ssrc: RtpSsrc)
        -> Result<Option<CsrcMapping>, MediaTransportError>;
    
    /// Get all CSRC mappings
    ///
    /// Returns all CSRC mappings currently registered.
    async fn get_all_csrc_mappings(&self)
        -> Result<Vec<CsrcMapping>, MediaTransportError>;
    
    /// Get CSRC values for active sources
    ///
    /// Returns the CSRC values for the specified active SSRCs.
    ///
    /// - `active_ssrcs`: The list of active SSRCs to get CSRCs for
    async fn get_active_csrcs(&self, active_ssrcs: &[RtpSsrc])
        -> Result<Vec<RtpCsrc>, MediaTransportError>;
        
    // Header Extensions API Methods
    
    /// Check if header extensions are enabled
    ///
    /// Returns true if header extensions are enabled for this server.
    async fn is_header_extensions_enabled(&self) -> Result<bool, MediaTransportError>;
    
    /// Enable header extensions with the specified format
    ///
    /// This enables the header extensions feature if it was not enabled
    /// in the configuration. Returns true if successfully enabled.
    ///
    /// - `format`: The header extension format to use (One-byte or Two-byte)
    async fn enable_header_extensions(&self, format: ExtensionFormat) -> Result<bool, MediaTransportError>;
    
    /// Configure a header extension mapping
    ///
    /// Maps an extension ID to a URI that identifies its type.
    ///
    /// - `id`: The extension ID to map (1-14 for one-byte, 1-255 for two-byte)
    /// - `uri`: The URI that identifies this extension type
    async fn configure_header_extension(&self, id: u8, uri: String) 
        -> Result<(), MediaTransportError>;
        
    /// Configure multiple header extension mappings
    ///
    /// Maps extension IDs to URIs that identify their types.
    ///
    /// - `mappings`: A HashMap of extension IDs to URIs
    async fn configure_header_extensions(&self, mappings: HashMap<u8, String>)
        -> Result<(), MediaTransportError>;
    
    /// Add a header extension to the next outgoing packet for a specific client
    ///
    /// - `client_id`: The ID of the client to add the extension for
    /// - `extension`: The header extension to add
    async fn add_header_extension_for_client(&self, client_id: &str, extension: HeaderExtension)
        -> Result<(), MediaTransportError>;
    
    /// Add a header extension to the next outgoing packet for all clients
    ///
    /// - `extension`: The header extension to add
    async fn add_header_extension_for_all_clients(&self, extension: HeaderExtension)
        -> Result<(), MediaTransportError>;
    
    /// Add audio level header extension for a specific client
    ///
    /// - `client_id`: The ID of the client to add the extension for
    /// - `voice_activity`: true if voice activity is detected, false otherwise
    /// - `level`: audio level in dB below full scale (0-127)
    async fn add_audio_level_extension_for_client(&self, client_id: &str, voice_activity: bool, level: u8)
        -> Result<(), MediaTransportError>;
    
    /// Add audio level header extension for all clients
    ///
    /// - `voice_activity`: true if voice activity is detected, false otherwise
    /// - `level`: audio level in dB below full scale (0-127)
    async fn add_audio_level_extension_for_all_clients(&self, voice_activity: bool, level: u8)
        -> Result<(), MediaTransportError>;
    
    /// Add video orientation header extension for a specific client
    ///
    /// - `client_id`: The ID of the client to add the extension for
    /// - `camera_front_facing`: true if camera is front-facing, false otherwise
    /// - `camera_flipped`: true if camera is flipped, false otherwise
    /// - `rotation`: rotation in degrees (0, 90, 180, or 270)
    async fn add_video_orientation_extension_for_client(&self, client_id: &str, 
        camera_front_facing: bool, camera_flipped: bool, rotation: u16)
        -> Result<(), MediaTransportError>;
    
    /// Add video orientation header extension for all clients
    ///
    /// - `camera_front_facing`: true if camera is front-facing, false otherwise
    /// - `camera_flipped`: true if camera is flipped, false otherwise
    /// - `rotation`: rotation in degrees (0, 90, 180, or 270)
    async fn add_video_orientation_extension_for_all_clients(&self, 
        camera_front_facing: bool, camera_flipped: bool, rotation: u16)
        -> Result<(), MediaTransportError>;
    
    /// Add transport-cc header extension for a specific client
    ///
    /// - `client_id`: The ID of the client to add the extension for
    /// - `sequence_number`: transport-wide sequence number
    async fn add_transport_cc_extension_for_client(&self, client_id: &str, sequence_number: u16)
        -> Result<(), MediaTransportError>;
    
    /// Add transport-cc header extension for all clients
    ///
    /// - `sequence_number`: transport-wide sequence number
    async fn add_transport_cc_extension_for_all_clients(&self, sequence_number: u16)
        -> Result<(), MediaTransportError>;
    
    /// Get all header extensions received from a specific client
    ///
    /// - `client_id`: The ID of the client to get extensions from
    async fn get_received_header_extensions(&self, client_id: &str)
        -> Result<Vec<HeaderExtension>, MediaTransportError>;
    
    /// Get audio level header extension from a specific client
    ///
    /// - `client_id`: The ID of the client to get the extension from
    ///
    /// Returns a tuple of (voice_activity, level) if the extension is found
    async fn get_received_audio_level(&self, client_id: &str)
        -> Result<Option<(bool, u8)>, MediaTransportError>;
    
    /// Get video orientation header extension from a specific client
    ///
    /// - `client_id`: The ID of the client to get the extension from
    ///
    /// Returns a tuple of (camera_front_facing, camera_flipped, rotation) if the extension is found
    async fn get_received_video_orientation(&self, client_id: &str)
        -> Result<Option<(bool, bool, u16)>, MediaTransportError>;
    
    /// Get transport-cc header extension from a specific client
    ///
    /// - `client_id`: The ID of the client to get the extension from
    ///
    /// Returns the transport-wide sequence number if the extension is found
    async fn get_received_transport_cc(&self, client_id: &str)
        -> Result<Option<u16>, MediaTransportError>;
} 