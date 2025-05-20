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
    
    /// Send an RTCP Receiver Report
    ///
    /// This sends a Receiver Report RTCP packet to the remote peer. This can be
    /// useful to force an immediate quality report instead of waiting for the
    /// automatic interval-based reports.
    async fn send_rtcp_receiver_report(&self) -> Result<(), MediaTransportError>;
    
    /// Send an RTCP Sender Report
    ///
    /// This sends a Sender Report RTCP packet to the remote peer. This can be
    /// useful to force an immediate quality report instead of waiting for the
    /// automatic interval-based reports.
    async fn send_rtcp_sender_report(&self) -> Result<(), MediaTransportError>;
    
    /// Get detailed RTCP statistics
    ///
    /// This returns detailed quality metrics gathered from RTCP reports
    /// including jitter, packet loss, and round-trip time.
    async fn get_rtcp_stats(&self) -> Result<RtcpStats, MediaTransportError>;
    
    /// Set the RTCP report interval
    ///
    /// This sets how frequently RTCP reports are sent. The default is usually
    /// 5% of the session bandwidth, but this can be adjusted for more or less
    /// frequent reporting.
    async fn set_rtcp_interval(&self, interval: Duration) -> Result<(), MediaTransportError>;
    
    /// Send an RTCP Application-Defined (APP) packet
    ///
    /// This sends an RTCP APP packet with the specified name and application data.
    /// APP packets are used for application-specific purposes and allow
    /// custom data to be exchanged between endpoints.
    ///
    /// - `name`: A four-character ASCII name to identify the application
    /// - `data`: The application-specific data to send
    async fn send_rtcp_app(&self, name: &str, data: Vec<u8>) -> Result<(), MediaTransportError>;
    
    /// Send an RTCP Goodbye (BYE) packet
    ///
    /// This sends an RTCP BYE packet with an optional reason for leaving.
    /// BYE packets are used to indicate that a source is no longer active.
    ///
    /// - `reason`: An optional reason string for leaving
    async fn send_rtcp_bye(&self, reason: Option<String>) -> Result<(), MediaTransportError>;
    
    /// Send an RTCP Extended Report (XR) packet with VoIP metrics
    ///
    /// This sends an RTCP XR packet with VoIP metrics for the specified SSRC.
    /// XR packets are used to report extended statistics beyond what is
    /// available in standard Sender/Receiver Reports.
    ///
    /// - `metrics`: The VoIP metrics to include in the XR packet
    async fn send_rtcp_xr_voip_metrics(&self, metrics: VoipMetrics) -> Result<(), MediaTransportError>;
}

// Re-export the implementation
pub use client_transport_impl::DefaultMediaTransportClient;

/// RTCP Statistics 
#[derive(Debug, Clone, Default)]
pub struct RtcpStats {
    /// Jitter (in milliseconds)
    pub jitter_ms: f64,
    
    /// Packet loss percentage (0.0 - 100.0)
    pub packet_loss_percent: f64,
    
    /// Round-trip time (in milliseconds, if available)
    pub round_trip_time_ms: Option<f64>,
    
    /// Number of RTCP packets sent
    pub rtcp_packets_sent: u64,
    
    /// Number of RTCP packets received
    pub rtcp_packets_received: u64,
    
    /// Timestamp of last RTCP Sender Report received
    pub last_sr_timestamp: Option<u64>,
    
    /// Timestamp of last RTCP Receiver Report received
    pub last_rr_timestamp: Option<u64>,
    
    /// Cumulative number of packets lost
    pub cumulative_packets_lost: u32,
}

/// VoIP Metrics for RTCP XR
#[derive(Debug, Clone)]
pub struct VoipMetrics {
    /// SSRC of the stream this metrics belongs to
    pub ssrc: u32,
    
    /// Packet loss rate in percent (0-255)
    pub loss_rate: u8,
    
    /// Packet discard rate in percent (0-255)
    pub discard_rate: u8,
    
    /// Burst density in percent (0-255)
    pub burst_density: u8,
    
    /// Gap density in percent (0-255)
    pub gap_density: u8,
    
    /// Burst duration in milliseconds
    pub burst_duration: u16,
    
    /// Gap duration in milliseconds
    pub gap_duration: u16,
    
    /// Round trip delay in milliseconds
    pub round_trip_delay: u16,
    
    /// End system delay in milliseconds
    pub end_system_delay: u16,
    
    /// Signal level in dBm (-127 to 0)
    pub signal_level: i8,
    
    /// Noise level in dBm (-127 to 0)
    pub noise_level: i8,
    
    /// Residual Echo Return Loss in dB (0-255)
    pub rerl: u8,
    
    /// R-factor (listening quality)
    pub r_factor: u8,
    
    /// MOS-LQ (listening quality MOS, 10-50, representing 1.0 to 5.0)
    pub mos_lq: u8,
    
    /// MOS-CQ (conversational quality MOS, 10-50, representing 1.0 to 5.0)
    pub mos_cq: u8,
    
    /// Jitter buffer nominal delay in milliseconds
    pub jb_nominal: u16,
    
    /// Jitter buffer maximum delay in milliseconds
    pub jb_maximum: u16,
    
    /// Jitter buffer absolute maximum delay in milliseconds
    pub jb_abs_max: u16,
} 