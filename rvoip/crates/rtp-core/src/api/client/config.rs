//! Client configuration
//!
//! This module contains client-specific configuration types and builders.

use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;
use crate::api::common::frame::MediaFrameType;
use crate::api::common::error::MediaTransportError;
use crate::api::common::config::{BaseTransportConfig, SecurityMode, SrtpProfile};
use crate::transport::{PortAllocator, GlobalPortAllocator};

/// Client transport configuration
#[derive(Debug, Clone)]
pub struct ClientConfig {
    /// Base transport configuration
    pub base: BaseTransportConfig,
    /// Remote address to send to
    pub remote_address: Option<SocketAddr>,
    /// RTCP address to send to
    pub rtcp_address: Option<SocketAddr>,
    /// Security mode
    pub security_mode: SecurityMode,
    /// SRTP protection profiles in order of preference
    pub srtp_profiles: Vec<SrtpProfile>,
    /// Whether DTLS should take client role (true = client, false = server)
    pub dtls_client: bool,
    /// Pre-shared SRTP key material (only used in SrtpWithPsk mode)
    pub psk_material: Option<Vec<u8>>,
}

impl Default for ClientConfig {
    fn default() -> Self {
        Self {
            base: BaseTransportConfig {
                local_address: None,
                rtcp_mux: true,
                media_types: vec![MediaFrameType::Audio],
                mtu: 1200,
            },
            remote_address: None,
            rtcp_address: None,
            security_mode: SecurityMode::DtlsSrtp,
            srtp_profiles: vec![
                SrtpProfile::AesGcm128,
                SrtpProfile::AesCm128HmacSha1_80,
            ],
            dtls_client: true,
            psk_material: None,
        }
    }
}

/// Builder for ClientConfig
pub struct ClientConfigBuilder {
    /// Config being built
    pub config: ClientConfig,
}

impl ClientConfigBuilder {
    /// Create a new builder with default configuration
    pub fn new() -> Self {
        Self {
            config: ClientConfig::default(),
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
    
    /// Set the DTLS setup role (true = client, false = server)
    pub fn dtls_client(mut self, client: bool) -> Self {
        self.config.dtls_client = client;
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
        
        // If RTCP multiplexing is not enabled, ensure we have a separate RTCP port
        if !self.config.base.rtcp_mux && rtcp_addr.is_some() {
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
        if !self.config.base.rtcp_mux && rtcp_addr.is_some() {
            self = self.rtcp_address(rtcp_addr.unwrap());
        }
        
        Ok(self)
    }
    
    /// Build the configuration
    pub fn build(self) -> Result<ClientConfig, MediaTransportError> {
        // Validate the configuration
        if self.config.base.local_address.is_none() {
            return Err(MediaTransportError::ConfigError(
                "Local address is required".to_string(),
            ));
        }
        
        if self.config.remote_address.is_none() {
            return Err(MediaTransportError::ConfigError(
                "Remote address is required for client configuration".to_string(),
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