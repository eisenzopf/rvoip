//! Media Transport API
//!
//! This module provides an abstraction layer for media transport,
//! simplifying RTP/RTCP interactions for media-core integration.

use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use thiserror::Error;

// For internal integration
use crate::packet::rtp::RtpPacket;
use crate::session::RtpSession;
use crate::transport::RtpTransport;

/// Error types specific to media transport operations
#[derive(Error, Debug)]
pub enum MediaTransportError {
    /// Failed to send media frame
    #[error("Failed to send media frame: {0}")]
    SendError(String),
    
    /// Failed to receive media frame
    #[error("Failed to receive media frame: {0}")]
    ReceiveError(String),
    
    /// Transport connection error
    #[error("Transport connection error: {0}")]
    ConnectionError(String),
    
    /// Configuration error
    #[error("Configuration error: {0}")]
    ConfigurationError(String),
}

/// Media frame types that can be transported
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaFrameType {
    /// Audio frame
    Audio,
    /// Video frame
    Video,
    /// Data channel frame
    Data,
}

/// A media frame containing encoded media data
#[derive(Debug)]
pub struct MediaFrame {
    /// The type of media frame
    pub frame_type: MediaFrameType,
    /// The payload data
    pub data: Vec<u8>,
    /// Timestamp in media clock units
    pub timestamp: u32,
    /// Sequence identifier for ordering
    pub sequence: u16,
    /// Marker bit (e.g., end of frame for video)
    pub marker: bool,
    /// Payload type identifier
    pub payload_type: u8,
    /// Synchronization source identifier
    pub ssrc: u32,
}

/// Media transport event types for notifications
#[derive(Debug, Clone)]
pub enum MediaTransportEvent {
    /// Transport connected successfully
    Connected,
    /// Transport disconnected
    Disconnected,
    /// Network quality changed
    QualityChanged {
        /// The new quality level
        quality: crate::api::stats::QualityLevel,
    },
    /// New bandwidth estimate available
    BandwidthEstimate {
        /// Estimated available bandwidth in bits per second
        bps: u32,
    },
    /// Remote address changed (e.g., ICE candidate switch)
    RemoteAddressChanged {
        /// The new remote address
        address: SocketAddr,
    },
}

/// Callback for receiving transport events
pub type MediaEventCallback = Box<dyn Fn(MediaTransportEvent) + Send + Sync>;

/// Configuration for media transport
#[derive(Debug, Clone)]
pub struct MediaTransportConfig {
    /// Local address to bind to
    pub local_address: SocketAddr,
    /// Remote address to send to
    pub remote_address: Option<SocketAddr>,
    /// Whether to use RTCP multiplexing (RTP and RTCP on same port)
    pub rtcp_mux: bool,
    /// Media types enabled for this transport
    pub media_types: Vec<MediaFrameType>,
    /// Maximum transmission unit size
    pub mtu: usize,
}

/// Builder for MediaTransportConfig
#[derive(Default)]
pub struct MediaTransportConfigBuilder {
    config: Option<MediaTransportConfig>,
}

impl MediaTransportConfigBuilder {
    /// Create a new builder
    pub fn new() -> Self {
        Self { config: None }
    }
    
    /// Set the local address
    pub fn local_address(mut self, addr: SocketAddr) -> Self {
        let config = self.config.get_or_insert(MediaTransportConfig {
            local_address: addr,
            remote_address: None,
            rtcp_mux: true,
            media_types: vec![MediaFrameType::Audio],
            mtu: 1200,
        });
        config.local_address = addr;
        self
    }
    
    /// Set the remote address
    pub fn remote_address(mut self, addr: SocketAddr) -> Self {
        if let Some(config) = self.config.as_mut() {
            config.remote_address = Some(addr);
        }
        self
    }
    
    /// Enable or disable RTCP multiplexing
    pub fn rtcp_mux(mut self, enabled: bool) -> Self {
        if let Some(config) = self.config.as_mut() {
            config.rtcp_mux = enabled;
        }
        self
    }
    
    /// Set the media types enabled for this transport
    pub fn media_types(mut self, types: Vec<MediaFrameType>) -> Self {
        if let Some(config) = self.config.as_mut() {
            config.media_types = types;
        }
        self
    }
    
    /// Set the MTU size
    pub fn mtu(mut self, mtu: usize) -> Self {
        if let Some(config) = self.config.as_mut() {
            config.mtu = mtu;
        }
        self
    }
    
    /// Build the configuration
    pub fn build(self) -> Result<MediaTransportConfig, MediaTransportError> {
        self.config.ok_or_else(|| MediaTransportError::ConfigurationError(
            "Local address must be specified".to_string()
        ))
    }
}

/// The main interface for media transport
///
/// This trait provides the primary integration point for media-core to interact
/// with the RTP-based transport layer. It abstracts away packet-level details
/// and provides a frame-oriented interface.
#[async_trait]
pub trait MediaTransportSession: Send + Sync {
    /// Start the transport session
    async fn start(&self) -> Result<(), MediaTransportError>;
    
    /// Stop the transport session
    async fn stop(&self) -> Result<(), MediaTransportError>;
    
    /// Send a media frame
    async fn send_frame(&self, frame: MediaFrame) -> Result<(), MediaTransportError>;
    
    /// Receive a media frame
    ///
    /// This method returns the next available media frame, or None if no frame
    /// is available within the specified timeout.
    async fn receive_frame(&self, timeout: Duration) -> Result<Option<MediaFrame>, MediaTransportError>;
    
    /// Set the remote address
    ///
    /// This can be used to update the remote address after ICE negotiation.
    async fn set_remote_address(&self, addr: SocketAddr) -> Result<(), MediaTransportError>;
    
    /// Register a callback for transport events
    fn on_event(&self, callback: MediaEventCallback) -> Result<(), MediaTransportError>;
    
    /// Get current transport statistics
    async fn get_stats(&self) -> Result<crate::api::stats::MediaStats, MediaTransportError>;
}

/// Factory for creating MediaTransportSession instances
pub struct MediaTransportFactory;

impl MediaTransportFactory {
    /// Create a new MediaTransportSession
    pub async fn create_session(
        config: MediaTransportConfig,
        security_config: Option<crate::api::security::SecurityConfig>,
        buffer_config: Option<crate::api::buffer::MediaBufferConfig>,
    ) -> Result<Arc<dyn MediaTransportSession>, MediaTransportError> {
        // This is a placeholder that will be implemented to create the actual transport session
        // based on the internal RtpSession and other components
        todo!("Implement session creation using internal components")
    }
} 