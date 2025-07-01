//! RTP engine configuration and component definitions

use async_trait::async_trait;
use serde::{Serialize, Deserialize};
use std::net::SocketAddr;

use crate::{VoipBuilderError, ComponentStatus, SecurityConfig};

/// RTP engine configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RtpEngineConfig {
    /// RTP bind address range
    pub rtp_port_range: (u16, u16),
    /// Security configuration
    pub security: SecurityConfig,
    /// Quality of Service settings
    pub qos_enabled: bool,
}

impl RtpEngineConfig {
    /// Create a basic RTP engine configuration
    pub fn basic() -> Self {
        Self {
            rtp_port_range: (10000, 20000),
            security: SecurityConfig::Auto,
            qos_enabled: false,
        }
    }

    /// Create a secure RTP engine configuration
    pub fn secure() -> Self {
        Self {
            rtp_port_range: (10000, 20000),
            security: SecurityConfig::DtlsSrtp,
            qos_enabled: true,
        }
    }
}

/// Trait for RTP engine components
#[async_trait]
pub trait RtpEngineComponent: Send + Sync + std::fmt::Debug {
    /// Start the RTP engine
    async fn start(&mut self) -> Result<(), VoipBuilderError>;
    
    /// Stop the RTP engine
    async fn stop(&mut self) -> Result<(), VoipBuilderError>;
    
    /// Get component health status
    async fn health(&self) -> ComponentStatus;
    
    /// Get component configuration
    fn config(&self) -> &RtpEngineConfig;
} 