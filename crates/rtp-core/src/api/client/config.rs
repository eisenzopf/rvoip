//! Client configuration
//!
//! This module defines client-specific configuration types.

use std::net::SocketAddr;
use crate::api::client::security::ClientSecurityConfig;
use crate::api::common::extension::ExtensionFormat;
use crate::buffer::{TransmitBufferConfig, BufferLimits};

/// Client configuration
#[derive(Debug, Clone)]
pub struct ClientConfig {
    /// Remote address to connect to
    pub remote_address: Option<SocketAddr>,
    /// Local address to connect to
    pub local_address: Option<SocketAddr>,
    /// Default payload type
    pub default_payload_type: u8,
    /// Clock rate in Hz
    pub clock_rate: u32,
    /// Security configuration
    pub security_config: ClientSecurityConfig,
    /// Jitter buffer size in packets
    pub jitter_buffer_size: u16,
    /// Maximum packet age in milliseconds
    pub jitter_max_packet_age_ms: u16,
    /// Enable jitter buffer
    pub enable_jitter_buffer: bool,
    /// Local SSRC
    pub ssrc: Option<u32>,
    /// Enable RTCP multiplexing (RFC 5761)
    pub rtcp_mux: bool,
    /// Enable media synchronization features (optional)
    pub media_sync_enabled: Option<bool>,
    /// Enable SSRC demultiplexing for handling multiple streams
    pub ssrc_demultiplexing_enabled: Option<bool>,
    /// Enable CSRC management for conferencing scenarios
    pub csrc_management_enabled: bool,
    /// Enable header extensions support (RFC 8285)
    pub header_extensions_enabled: bool,
    /// Header extension format (One-byte or Two-byte)
    pub header_extension_format: ExtensionFormat,
    /// Transmit buffer configuration
    pub transmit_buffer_config: TransmitBufferConfig,
    /// Buffer limits
    pub buffer_limits: BufferLimits,
    /// Enable high-performance buffers
    pub high_performance_buffers_enabled: bool,
}

/// Builder for ClientConfig
#[derive(Debug, Clone)]
pub struct ClientConfigBuilder {
    /// Client configuration being built
    config: ClientConfig,
}

impl ClientConfigBuilder {
    /// Create a new client config builder with default values
    pub fn new() -> Self {
        Self {
            config: ClientConfig::default(),
        }
    }
    
    /// Create a builder with WebRTC-optimized defaults
    pub fn webrtc() -> Self {
        let mut builder = Self::new();
        builder.config.security_config.security_mode = crate::api::common::config::SecurityMode::DtlsSrtp;
        builder.config.rtcp_mux = true; // WebRTC typically uses RTCP-MUX
        builder.config.header_extensions_enabled = true; // WebRTC makes extensive use of header extensions
        builder
    }
    
    /// Create a builder with SIP-optimized defaults
    pub fn sip() -> Self {
        let mut builder = Self::new();
        builder.config.security_config.security_mode = crate::api::common::config::SecurityMode::Srtp;
        builder.config.rtcp_mux = false; // Traditional SIP doesn't use RTCP-MUX by default
        builder
    }
    
    /// Set the remote address
    pub fn remote_address(mut self, addr: SocketAddr) -> Self {
        self.config.remote_address = Some(addr);
        self
    }
    
    /// Set the remote address to localhost (for testing only)
    pub fn remote_address_localhost(mut self, port: u16) -> Self {
        self.config.remote_address = Some(SocketAddr::from(([127, 0, 0, 1], port)));
        self
    }
    
    /// Set the local address
    pub fn local_address(mut self, addr: Option<SocketAddr>) -> Self {
        self.config.local_address = addr;
        self
    }
    
    /// Set the default payload type
    pub fn default_payload_type(mut self, pt: u8) -> Self {
        self.config.default_payload_type = pt;
        self
    }
    
    /// Set the clock rate
    pub fn clock_rate(mut self, rate: u32) -> Self {
        self.config.clock_rate = rate;
        self
    }
    
    /// Set the security configuration
    pub fn security_config(mut self, config: ClientSecurityConfig) -> Self {
        self.config.security_config = config;
        self
    }
    
    /// Set the jitter buffer size
    pub fn jitter_buffer_size(mut self, size: u16) -> Self {
        self.config.jitter_buffer_size = size;
        self
    }
    
    /// Set the maximum packet age
    pub fn jitter_max_packet_age_ms(mut self, age: u16) -> Self {
        self.config.jitter_max_packet_age_ms = age;
        self
    }
    
    /// Enable or disable the jitter buffer
    pub fn enable_jitter_buffer(mut self, enable: bool) -> Self {
        self.config.enable_jitter_buffer = enable;
        self
    }
    
    /// Set the SSRC
    pub fn ssrc(mut self, ssrc: u32) -> Self {
        self.config.ssrc = Some(ssrc);
        self
    }
    
    /// Enable or disable RTCP multiplexing (RFC 5761)
    pub fn rtcp_mux(mut self, enable: bool) -> Self {
        self.config.rtcp_mux = enable;
        self
    }
    
    /// Enable or disable media synchronization features
    pub fn media_sync_enabled(mut self, enable: bool) -> Self {
        self.config.media_sync_enabled = Some(enable);
        self
    }
    
    /// Enable or disable SSRC demultiplexing for handling multiple streams
    pub fn ssrc_demultiplexing_enabled(mut self, enable: bool) -> Self {
        self.config.ssrc_demultiplexing_enabled = Some(enable);
        self
    }
    
    /// Enable or disable CSRC management for conferencing scenarios
    pub fn csrc_management_enabled(mut self, enable: bool) -> Self {
        self.config.csrc_management_enabled = enable;
        self
    }
    
    /// Enable or disable header extensions support (RFC 8285)
    pub fn header_extensions_enabled(mut self, enable: bool) -> Self {
        self.config.header_extensions_enabled = enable;
        self
    }
    
    /// Set the header extension format (One-byte or Two-byte)
    pub fn header_extension_format(mut self, format: ExtensionFormat) -> Self {
        self.config.header_extension_format = format;
        self
    }
    
    /// Set the transmit buffer configuration
    pub fn transmit_buffer_config(mut self, config: TransmitBufferConfig) -> Self {
        self.config.transmit_buffer_config = config;
        self
    }
    
    /// Set the buffer limits
    pub fn buffer_limits(mut self, limits: BufferLimits) -> Self {
        self.config.buffer_limits = limits;
        self
    }
    
    /// Enable or disable high-performance buffers
    pub fn high_performance_buffers_enabled(mut self, enabled: bool) -> Self {
        self.config.high_performance_buffers_enabled = enabled;
        self
    }
    
    /// Build the client configuration
    pub fn build(self) -> ClientConfig {
        self.config
    }
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            remote_address: None,
            local_address: None,
            default_payload_type: 0,
            clock_rate: 8000,
            security_config: ClientSecurityConfig::default(),
            jitter_buffer_size: 50,
            jitter_max_packet_age_ms: 200,
            enable_jitter_buffer: true,
            ssrc: None,
            rtcp_mux: false,
            media_sync_enabled: None,
            ssrc_demultiplexing_enabled: None,
            csrc_management_enabled: false,
            header_extensions_enabled: false,
            header_extension_format: ExtensionFormat::OneByte,
            transmit_buffer_config: TransmitBufferConfig::default(),
            buffer_limits: BufferLimits {
                max_packets_per_stream: 500,
                max_packet_size: 1500,
                max_memory: 10 * 1024 * 1024, // 10 MB default
            },
            high_performance_buffers_enabled: false,
        }
    }
} 