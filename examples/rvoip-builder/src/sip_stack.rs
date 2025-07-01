//! SIP stack configuration and component definitions

use async_trait::async_trait;
use serde::{Serialize, Deserialize};
use std::net::SocketAddr;

use crate::{VoipBuilderError, ComponentStatus};

/// SIP stack configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SipStackConfig {
    /// SIP server bind address
    pub bind_address: SocketAddr,
    /// Supported SIP transports
    pub transports: Vec<SipTransport>,
    /// Maximum concurrent connections
    pub max_connections: u32,
    /// Connection timeout
    pub connection_timeout: std::time::Duration,
    /// Enable SIP compression
    pub compression: bool,
    /// Custom SIP headers to add
    pub custom_headers: std::collections::HashMap<String, String>,
}

/// SIP transport protocols
#[derive(Debug, Clone, Copy, Serialize, Deserialize, PartialEq, Eq)]
pub enum SipTransport {
    Udp,
    Tcp,
    Tls,
    Ws,   // WebSocket
    Wss,  // WebSocket Secure
}

impl SipStackConfig {
    /// Create a basic SIP stack configuration
    pub fn basic() -> Self {
        Self {
            bind_address: "0.0.0.0:5060".parse().unwrap(),
            transports: vec![SipTransport::Udp, SipTransport::Tcp],
            max_connections: 1000,
            connection_timeout: std::time::Duration::from_secs(30),
            compression: false,
            custom_headers: std::collections::HashMap::new(),
        }
    }

    /// Create a WebRTC-compatible SIP stack configuration
    pub fn webrtc() -> Self {
        Self {
            bind_address: "0.0.0.0:8080".parse().unwrap(),
            transports: vec![SipTransport::Ws, SipTransport::Wss],
            max_connections: 10000,
            connection_timeout: std::time::Duration::from_secs(60),
            compression: true,
            custom_headers: std::collections::HashMap::new(),
        }
    }

    /// Create a custom SIP stack configuration
    pub fn custom() -> Self {
        Self::basic()
    }

    /// Create a secure SIP stack configuration
    pub fn secure() -> Self {
        Self {
            bind_address: "0.0.0.0:5061".parse().unwrap(),
            transports: vec![SipTransport::Tls, SipTransport::Wss],
            max_connections: 5000,
            connection_timeout: std::time::Duration::from_secs(45),
            compression: true,
            custom_headers: std::collections::HashMap::new(),
        }
    }
}

/// Trait for SIP stack components
#[async_trait]
pub trait SipStackComponent: Send + Sync + std::fmt::Debug {
    /// Start the SIP stack
    async fn start(&mut self) -> Result<(), VoipBuilderError>;
    
    /// Stop the SIP stack
    async fn stop(&mut self) -> Result<(), VoipBuilderError>;
    
    /// Get component health status
    async fn health(&self) -> ComponentStatus;
    
    /// Get component configuration
    fn config(&self) -> &SipStackConfig;
    
    /// Handle incoming SIP message
    async fn handle_message(&self, message: SipMessage) -> Result<(), VoipBuilderError>;
}

/// SIP message representation (placeholder)
#[derive(Debug, Clone)]
pub struct SipMessage {
    /// SIP method (INVITE, REGISTER, etc.)
    pub method: String,
    /// Request URI
    pub uri: String,
    /// SIP headers
    pub headers: std::collections::HashMap<String, String>,
    /// Message body
    pub body: Option<String>,
}

impl Default for SipStackConfig {
    fn default() -> Self {
        Self::basic()
    }
} 