//! Media Transport API
//!
//! This module provides an abstraction layer for media transport,
//! simplifying RTP/RTCP interactions for media-core integration.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use thiserror::Error;

// For internal integration
use crate::packet::rtp::RtpPacket;
use crate::session::RtpSession;
use crate::transport::RtpTransport;
use crate::transport::{PortAllocator, PortAllocatorConfig, GlobalPortAllocator, AllocationStrategy, PairingStrategy};

// Implementation module
mod media_transport_impl;

// Re-export implementation
pub use media_transport_impl::DefaultMediaTransportSession;

/// Error types for media transport
#[derive(Debug, thiserror::Error)]
pub enum MediaTransportError {
    /// Configuration error
    #[error("Configuration error: {0}")]
    ConfigError(String),
    
    /// Initialization error
    #[error("Initialization error: {0}")]
    InitializationError(String),
    
    /// Connection error
    #[error("Connection error: {0}")]
    ConnectionError(String),
    
    /// Authentication error
    #[error("Authentication error: {0}")]
    AuthenticationError(String),
    
    /// Packet send error
    #[error("Send error: {0}")]
    SendError(String),
    
    /// Packet receive error
    #[error("Receive error: {0}")]
    ReceiveError(String),
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
#[derive(Debug, Clone)]
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
    pub local_address: Option<SocketAddr>,
    /// Remote address to send to
    pub remote_address: Option<SocketAddr>,
    /// RTCP address to send to
    pub rtcp_address: Option<SocketAddr>,
    /// Whether to use RTCP multiplexing (RTP and RTCP on same port)
    pub rtcp_mux: bool,
    /// Media types enabled for this transport
    pub media_types: Vec<MediaFrameType>,
    /// Maximum transmission unit size
    pub mtu: usize,
}

/// Builder for MediaTransportConfig
pub struct MediaTransportConfigBuilder {
    /// Configuration being built
    pub config: MediaTransportConfig,
}

impl MediaTransportConfigBuilder {
    /// Create a new MediaTransportConfigBuilder
    pub fn new() -> Self {
        Self {
            config: MediaTransportConfig {
                local_address: None,
                remote_address: None,
                rtcp_address: None,
                rtcp_mux: true,
                media_types: vec![MediaFrameType::Audio],
                mtu: 1200,
            },
        }
    }
    
    /// Set local address
    pub fn local_address(mut self, addr: SocketAddr) -> Self {
        self.config.local_address = Some(addr);
        self
    }
    
    /// Set remote address
    pub fn remote_address(mut self, addr: SocketAddr) -> Self {
        self.config.remote_address = Some(addr);
        self
    }
    
    /// Set RTCP address
    pub fn rtcp_address(mut self, addr: SocketAddr) -> Self {
        self.config.rtcp_address = Some(addr);
        self
    }
    
    /// Set RTCP multiplexing (RTP and RTCP on same socket)
    pub fn rtcp_mux(mut self, enabled: bool) -> Self {
        self.config.rtcp_mux = enabled;
        self
    }
    
    /// Set media types supported by this transport
    pub fn media_types(mut self, types: Vec<MediaFrameType>) -> Self {
        self.config.media_types = types;
        self
    }
    
    /// Set maximum transmission unit (MTU)
    pub fn mtu(mut self, mtu: usize) -> Self {
        self.config.mtu = mtu;
        self
    }
    
    /// Use the port allocator to dynamically allocate ports
    pub async fn with_dynamic_ports(mut self, session_id: &str, ip: Option<IpAddr>) -> Result<Self, MediaTransportError> {
        // Get the global port allocator instance
        let allocator = GlobalPortAllocator::instance().await;
        
        // Allocate a pair of ports
        let (rtp_addr, rtcp_addr) = allocator.allocate_port_pair(session_id, ip)
            .await
            .map_err(|e| MediaTransportError::ConfigError(format!("Failed to allocate ports: {}", e)))?;
        
        // Update the configuration with the allocated ports
        self = self.local_address(rtp_addr);
        
        // If RTCP multiplexing is not enabled, ensure we have a separate RTCP port
        if !self.config.rtcp_mux && rtcp_addr.is_some() {
            self = self.rtcp_address(rtcp_addr.unwrap());
        }
        
        Ok(self)
    }
    
    /// Use a specific port allocator instance to allocate ports
    pub async fn with_port_allocator(
        mut self, 
        allocator: Arc<PortAllocator>, 
        session_id: &str, 
        ip: Option<IpAddr>
    ) -> Result<Self, MediaTransportError> {
        // Allocate a pair of ports
        let (rtp_addr, rtcp_addr) = allocator.allocate_port_pair(session_id, ip)
            .await
            .map_err(|e| MediaTransportError::ConfigError(format!("Failed to allocate ports: {}", e)))?;
        
        // Update the configuration with the allocated ports
        self = self.local_address(rtp_addr);
        
        // If RTCP multiplexing is not enabled, ensure we have a separate RTCP port
        if !self.config.rtcp_mux && rtcp_addr.is_some() {
            self = self.rtcp_address(rtcp_addr.unwrap());
        }
        
        Ok(self)
    }
    
    /// Build the configuration
    pub fn build(self) -> Result<MediaTransportConfig, MediaTransportError> {
        // Validate the configuration
        if self.config.local_address.is_none() {
            return Err(MediaTransportError::ConfigError(
                "Local address is required".to_string(),
            ));
        }
        
        Ok(self.config)
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
    
    /// Get security information for SDP exchange
    ///
    /// This returns security information like DTLS fingerprint that can be
    /// included in SDP for secure media negotiation.
    async fn get_security_info(&self) -> Result<crate::api::security::SecurityInfo, MediaTransportError>;
    
    /// Set the remote fingerprint for DTLS-SRTP
    ///
    /// This should be called with the fingerprint received from the remote peer's SDP
    /// before starting the transport session when using DTLS-SRTP.
    async fn set_remote_fingerprint(&self, fingerprint: &str, algorithm: &str) -> Result<(), MediaTransportError>;
}

/// Factory for creating media transport sessions
pub struct MediaTransportFactory; 