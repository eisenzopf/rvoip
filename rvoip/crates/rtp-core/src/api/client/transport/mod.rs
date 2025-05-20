//! Client transport API
//!
//! This module provides the client-specific transport interface for media transport.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;
use std::collections::HashMap;
use async_trait::async_trait;

use crate::api::common::frame::MediaFrame;
use crate::api::common::error::MediaTransportError;
use crate::api::common::events::MediaEventCallback;
use crate::api::client::config::ClientConfig;
use crate::api::common::config::SecurityInfo;
use crate::api::common::stats::MediaStats;
use crate::api::common::extension::ExtensionFormat;
use crate::packet::rtcp::NtpTimestamp;
use crate::{CsrcMapping, RtpSsrc, RtpCsrc};
use crate::api::server::transport::HeaderExtension;

pub mod client_transport_impl;

/// Media Synchronization Information
#[derive(Debug, Clone)]
pub struct MediaSyncInfo {
    /// SSRC of the stream
    pub ssrc: u32,
    
    /// Clock rate of the stream in Hz
    pub clock_rate: u32,
    
    /// Last NTP timestamp from RTCP SR
    pub last_ntp: Option<NtpTimestamp>,
    
    /// RTP timestamp corresponding to the last NTP timestamp
    pub last_rtp: Option<u32>,
    
    /// Measured clock drift in parts-per-million (positive means faster than reference)
    pub clock_drift_ppm: f64,
}

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
    
    /// Enable media synchronization
    ///
    /// This enables the media synchronization feature if it was not enabled
    /// in the configuration. Returns true if successfully enabled.
    async fn enable_media_sync(&self) -> Result<bool, MediaTransportError>;
    
    /// Check if media synchronization is enabled
    ///
    /// Returns true if media synchronization is enabled.
    async fn is_media_sync_enabled(&self) -> Result<bool, MediaTransportError>;
    
    /// Register a stream for synchronization
    ///
    /// - `ssrc`: The SSRC of the stream to register
    /// - `clock_rate`: The clock rate of the stream in Hz
    async fn register_sync_stream(&self, ssrc: u32, clock_rate: u32) -> Result<(), MediaTransportError>;
    
    /// Set the reference stream for synchronization
    ///
    /// The reference stream is typically an audio stream, as audio
    /// sync issues are more noticeable than video sync issues.
    ///
    /// - `ssrc`: The SSRC of the stream to use as reference
    async fn set_sync_reference_stream(&self, ssrc: u32) -> Result<(), MediaTransportError>;
    
    /// Get synchronization information for a stream
    ///
    /// Returns the synchronization information for the specified stream.
    ///
    /// - `ssrc`: The SSRC of the stream to get information for
    async fn get_sync_info(&self, ssrc: u32) -> Result<Option<MediaSyncInfo>, MediaTransportError>;
    
    /// Get synchronization information for all registered streams
    ///
    /// Returns a map of SSRCs to their synchronization information.
    async fn get_all_sync_info(&self) -> Result<HashMap<u32, MediaSyncInfo>, MediaTransportError>;
    
    /// Convert an RTP timestamp from one stream to the equivalent timestamp in another stream
    ///
    /// This allows synchronizing media from different streams that use different clock rates.
    ///
    /// - `from_ssrc`: The SSRC of the source stream
    /// - `to_ssrc`: The SSRC of the destination stream
    /// - `rtp_ts`: The RTP timestamp to convert
    async fn convert_timestamp(&self, from_ssrc: u32, to_ssrc: u32, rtp_ts: u32) -> Result<Option<u32>, MediaTransportError>;
    
    /// Convert an RTP timestamp to an NTP timestamp
    ///
    /// This converts an RTP timestamp to an NTP timestamp, which can be used
    /// to calculate the wall clock time of a packet.
    ///
    /// - `ssrc`: The SSRC of the stream
    /// - `rtp_ts`: The RTP timestamp to convert
    async fn rtp_to_ntp(&self, ssrc: u32, rtp_ts: u32) -> Result<Option<NtpTimestamp>, MediaTransportError>;
    
    /// Convert an NTP timestamp to an RTP timestamp
    ///
    /// This converts an NTP timestamp to an RTP timestamp for a specific stream.
    ///
    /// - `ssrc`: The SSRC of the stream
    /// - `ntp`: The NTP timestamp to convert
    async fn ntp_to_rtp(&self, ssrc: u32, ntp: NtpTimestamp) -> Result<Option<u32>, MediaTransportError>;
    
    /// Get clock drift for a stream in parts per million
    ///
    /// Returns the clock drift for the specified stream in parts per million.
    /// Positive values mean the stream's clock is faster than the reference.
    ///
    /// - `ssrc`: The SSRC of the stream to get drift for
    async fn get_clock_drift_ppm(&self, ssrc: u32) -> Result<Option<f64>, MediaTransportError>;
    
    /// Check if two streams are sufficiently synchronized
    ///
    /// Returns `true` if the streams have valid synchronization info
    /// and their clocks are aligned within the specified tolerance.
    ///
    /// - `ssrc1`: The SSRC of the first stream
    /// - `ssrc2`: The SSRC of the second stream
    /// - `tolerance_ms`: The maximum acceptable difference in milliseconds
    async fn are_streams_synchronized(&self, ssrc1: u32, ssrc2: u32, tolerance_ms: f64) -> Result<bool, MediaTransportError>;
    
    // CSRC Management API Methods
    
    /// Check if CSRC management is enabled
    ///
    /// Returns true if CSRC management is enabled for this client.
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
    /// Returns true if header extensions are enabled for this client.
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
    
    /// Add a header extension to the next outgoing packet
    ///
    /// - `extension`: The header extension to add
    async fn add_header_extension(&self, extension: HeaderExtension)
        -> Result<(), MediaTransportError>;
    
    /// Add audio level header extension
    ///
    /// - `voice_activity`: true if voice activity is detected, false otherwise
    /// - `level`: audio level in dB below full scale (0-127)
    async fn add_audio_level_extension(&self, voice_activity: bool, level: u8)
        -> Result<(), MediaTransportError>;
    
    /// Add video orientation header extension
    ///
    /// - `camera_front_facing`: true if camera is front-facing, false otherwise
    /// - `camera_flipped`: true if camera is flipped, false otherwise
    /// - `rotation`: rotation in degrees (0, 90, 180, or 270)
    async fn add_video_orientation_extension(&self, camera_front_facing: bool, camera_flipped: bool, rotation: u16)
        -> Result<(), MediaTransportError>;
    
    /// Add transport-cc header extension
    ///
    /// - `sequence_number`: transport-wide sequence number
    async fn add_transport_cc_extension(&self, sequence_number: u16)
        -> Result<(), MediaTransportError>;
    
    /// Get all header extensions received from the server
    async fn get_received_header_extensions(&self)
        -> Result<Vec<HeaderExtension>, MediaTransportError>;
    
    /// Get audio level header extension received from the server
    ///
    /// Returns a tuple of (voice_activity, level) if the extension is found
    async fn get_received_audio_level(&self)
        -> Result<Option<(bool, u8)>, MediaTransportError>;
    
    /// Get video orientation header extension received from the server
    ///
    /// Returns a tuple of (camera_front_facing, camera_flipped, rotation) if the extension is found
    async fn get_received_video_orientation(&self)
        -> Result<Option<(bool, bool, u16)>, MediaTransportError>;
    
    /// Get transport-cc header extension received from the server
    ///
    /// Returns the transport-wide sequence number if the extension is found
    async fn get_received_transport_cc(&self)
        -> Result<Option<u16>, MediaTransportError>;
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