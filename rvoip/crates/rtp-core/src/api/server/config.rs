//! Server configuration
//!
//! This module contains server-specific configuration types and builders.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use crate::api::common::frame::MediaFrameType;
use crate::api::common::error::MediaTransportError;
use crate::api::common::config::{BaseTransportConfig, SecurityMode, SrtpProfile};
use crate::transport::{PortAllocator, GlobalPortAllocator};

/// Server transport configuration
#[derive(Debug, Clone)]
pub struct ServerConfig {
    /// Base transport configuration
    pub base: BaseTransportConfig,
    /// Maximum number of concurrent clients
    pub max_clients: usize,
    /// Security mode
    pub security_mode: SecurityMode,
    /// SRTP protection profiles in order of preference
    pub srtp_profiles: Vec<SrtpProfile>,
    /// Whether to allow non-secure connections
    pub allow_insecure: bool,
    /// Pre-shared SRTP key material (only used in SrtpWithPsk mode)
    pub psk_material: Option<Vec<u8>>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            base: BaseTransportConfig {
                local_address: None,
                rtcp_mux: true,
                media_types: vec![MediaFrameType::Audio],
                mtu: 1200,
            },
            max_clients: 10,
            security_mode: SecurityMode::DtlsSrtp,
            srtp_profiles: vec![
                SrtpProfile::AesGcm128,
                SrtpProfile::AesCm128HmacSha1_80,
            ],
            allow_insecure: false,
            psk_material: None,
        }
    }
}

/// Builder for ServerConfig
pub struct ServerConfigBuilder {
    /// Config being built
    pub config: ServerConfig,
}

impl ServerConfigBuilder {
    /// Create a new builder with default configuration
    pub fn new() -> Self {
        Self {
            config: ServerConfig::default(),
        }
    }
    
    /// Create the WebRTC profile (optimized for WebRTC compatibility)
    pub fn webrtc() -> Self {
        let mut builder = Self::new();
        builder.config.srtp_profiles = vec![
            SrtpProfile::AesGcm128, 
            SrtpProfile::AesCm128HmacSha1_80,
        ];
        builder
    }
    
    /// Create the SIP profile (optimized for SIP compatibility)
    pub fn sip() -> Self {
        let mut builder = Self::new();
        builder.config.srtp_profiles = vec![
            SrtpProfile::AesCm128HmacSha1_80,
            SrtpProfile::AesCm128HmacSha1_32,
        ];
        builder
    }
    
    /// Set local address
    pub fn local_address(mut self, addr: SocketAddr) -> Self {
        self.config.base.local_address = Some(addr);
        self
    }
    
    /// Set RTCP multiplexing (RTP and RTCP on same socket)
    pub fn rtcp_mux(mut self, enabled: bool) -> Self {
        self.config.base.rtcp_mux = enabled;
        self
    }
    
    /// Set media types supported by this transport
    pub fn media_types(mut self, types: Vec<MediaFrameType>) -> Self {
        self.config.base.media_types = types;
        self
    }
    
    /// Set maximum transmission unit (MTU)
    pub fn mtu(mut self, mtu: usize) -> Self {
        self.config.base.mtu = mtu;
        self
    }
    
    /// Set maximum number of concurrent clients
    pub fn max_clients(mut self, max: usize) -> Self {
        self.config.max_clients = max;
        self
    }
    
    /// Set the security mode
    pub fn security_mode(mut self, mode: SecurityMode) -> Self {
        self.config.security_mode = mode;
        self
    }
    
    /// Set the SRTP protection profiles in order of preference
    pub fn srtp_profiles(mut self, profiles: Vec<SrtpProfile>) -> Self {
        self.config.srtp_profiles = profiles;
        self
    }
    
    /// Set whether to allow non-secure connections
    pub fn allow_insecure(mut self, allow: bool) -> Self {
        self.config.allow_insecure = allow;
        self
    }
    
    /// Set pre-shared key material for SRTP (only used in SrtpWithPsk mode)
    pub fn psk_material(mut self, material: Vec<u8>) -> Self {
        self.config.psk_material = Some(material);
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
        let (rtp_addr, _) = allocator.allocate_port_pair(session_id, ip)
            .await
            .map_err(|e| MediaTransportError::ConfigError(format!("Failed to allocate ports: {}", e)))?;
        
        // Update the configuration with the allocated ports
        self = self.local_address(rtp_addr);
        
        Ok(self)
    }
    
    /// Build the configuration
    pub fn build(self) -> Result<ServerConfig, MediaTransportError> {
        // Validate the configuration
        if self.config.base.local_address.is_none() {
            return Err(MediaTransportError::ConfigError(
                "Local address is required".to_string(),
            ));
        }
        
        if self.config.max_clients == 0 {
            return Err(MediaTransportError::ConfigError(
                "Maximum client count must be greater than zero".to_string(),
            ));
        }
        
        if self.config.security_mode == SecurityMode::SrtpWithPsk && self.config.psk_material.is_none() {
            return Err(MediaTransportError::ConfigError(
                "PSK material must be provided when using SrtpWithPsk mode".to_string(),
            ));
        }
        
        Ok(self.config)
    }
} 