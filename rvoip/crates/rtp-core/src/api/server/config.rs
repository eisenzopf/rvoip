//! Server configuration
//!
//! This module defines server-specific configuration types.

use std::net::SocketAddr;
use crate::api::server::security::ServerSecurityConfig;

/// Server configuration
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Local address to bind to
    pub local_address: SocketAddr,
    /// Default payload type
    pub default_payload_type: u8,
    /// Clock rate in Hz
    pub clock_rate: u32,
    /// Security configuration
    pub security_config: ServerSecurityConfig,
    /// Jitter buffer size in packets
    pub jitter_buffer_size: u32,
    /// Maximum packet age in milliseconds
    pub jitter_max_packet_age_ms: u32,
    /// Enable jitter buffer
    pub enable_jitter_buffer: bool,
    /// Maximum number of clients
    pub max_clients: usize,
    /// Enable RTCP multiplexing (RFC 5761)
    pub rtcp_mux: bool,
    /// Enable media synchronization features (optional)
    pub media_sync_enabled: Option<bool>,
    /// Enable SSRC demultiplexing for handling multiple streams
    pub ssrc_demultiplexing_enabled: Option<bool>,
}

/// Builder for ServerConfig
#[derive(Debug, Clone)]
pub struct ServerConfigBuilder {
    /// Server configuration being built
    config: ServerConfig,
}

impl ServerConfigBuilder {
    /// Create a new server config builder with default values
    pub fn new() -> Self {
        Self {
            config: ServerConfig {
                local_address: "0.0.0.0:0".parse().unwrap(),
                default_payload_type: 0,
                clock_rate: 8000,
                security_config: ServerSecurityConfig::default(),
                jitter_buffer_size: 100,
                jitter_max_packet_age_ms: 500,
                enable_jitter_buffer: true,
                max_clients: 100,
                rtcp_mux: false, // Disabled by default
                media_sync_enabled: None, // Optional, defaults to None
                ssrc_demultiplexing_enabled: None, // Optional, defaults to None
            },
        }
    }
    
    /// Create a builder with WebRTC-optimized defaults
    pub fn webrtc() -> Self {
        let mut builder = Self::new();
        builder.config.security_config.security_mode = crate::api::common::config::SecurityMode::DtlsSrtp;
        builder.config.rtcp_mux = true; // WebRTC typically uses RTCP-MUX
        builder
    }
    
    /// Create a builder with SIP-optimized defaults
    pub fn sip() -> Self {
        let mut builder = Self::new();
        builder.config.security_config.security_mode = crate::api::common::config::SecurityMode::Srtp;
        builder.config.rtcp_mux = false; // Traditional SIP doesn't use RTCP-MUX by default
        builder
    }
    
    /// Set the local address
    pub fn local_address(mut self, addr: SocketAddr) -> Self {
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
    pub fn security_config(mut self, config: ServerSecurityConfig) -> Self {
        self.config.security_config = config;
        self
    }
    
    /// Set the jitter buffer size
    pub fn jitter_buffer_size(mut self, size: u32) -> Self {
        self.config.jitter_buffer_size = size;
        self
    }
    
    /// Set the maximum packet age
    pub fn jitter_max_packet_age_ms(mut self, age: u32) -> Self {
        self.config.jitter_max_packet_age_ms = age;
        self
    }
    
    /// Enable or disable the jitter buffer
    pub fn enable_jitter_buffer(mut self, enable: bool) -> Self {
        self.config.enable_jitter_buffer = enable;
        self
    }
    
    /// Set the maximum number of clients
    pub fn max_clients(mut self, max: usize) -> Self {
        self.config.max_clients = max;
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
    
    /// Build the server configuration
    pub fn build(self) -> Result<ServerConfig, crate::api::common::error::MediaTransportError> {
        // Validate configuration
        if self.config.max_clients == 0 {
            return Err(crate::api::common::error::MediaTransportError::ConfigError(
                "Maximum number of clients cannot be zero".to_string(),
            ));
        }
        
        Ok(self.config)
    }
} 