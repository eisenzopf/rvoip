//! Server Configuration
//!
//! This module provides configuration types for the session-core server API.
//! It handles transport settings, protocol selection, and validation.

use std::net::SocketAddr;
use std::time::Duration;
use serde::{Deserialize, Serialize};
use anyhow::{Result, Context};

/// Transport protocol options for SIP server
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum TransportProtocol {
    /// UDP transport (RFC 3261)
    Udp,
    /// TCP transport (RFC 3261)
    Tcp,
    /// TLS transport (RFC 3261)
    Tls,
    /// WebSocket transport (RFC 7118)
    WebSocket,
}

impl Default for TransportProtocol {
    fn default() -> Self {
        Self::Udp
    }
}

impl std::fmt::Display for TransportProtocol {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            Self::Udp => write!(f, "UDP"),
            Self::Tcp => write!(f, "TCP"),
            Self::Tls => write!(f, "TLS"),
            Self::WebSocket => write!(f, "WS"),
        }
    }
}

/// Server configuration for session-core SIP server
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ServerConfig {
    /// Binding address and port for the server
    pub bind_address: SocketAddr,
    
    /// Transport protocol to use
    pub transport_protocol: TransportProtocol,
    
    /// Maximum number of concurrent sessions
    pub max_sessions: usize,
    
    /// Session timeout duration
    pub session_timeout: Duration,
    
    /// Transaction timeout duration
    pub transaction_timeout: Duration,
    
    /// Enable media coordination
    pub enable_media: bool,
    
    /// Server display name for SIP headers
    pub server_name: String,
    
    /// Contact URI for the server
    pub contact_uri: Option<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind_address: "127.0.0.1:5060".parse().unwrap(),
            transport_protocol: TransportProtocol::Udp,
            max_sessions: 1000,
            session_timeout: Duration::from_secs(300), // 5 minutes
            transaction_timeout: Duration::from_secs(32), // RFC 3261 Timer B
            enable_media: true,
            server_name: "rvoip-session-core".to_string(),
            contact_uri: None,
        }
    }
}

impl ServerConfig {
    /// Create a new server configuration
    pub fn new(bind_address: SocketAddr) -> Self {
        Self {
            bind_address,
            ..Default::default()
        }
    }
    
    /// Set the transport protocol
    pub fn with_transport(mut self, protocol: TransportProtocol) -> Self {
        self.transport_protocol = protocol;
        self
    }
    
    /// Set the maximum number of sessions
    pub fn with_max_sessions(mut self, max_sessions: usize) -> Self {
        self.max_sessions = max_sessions;
        self
    }
    
    /// Set the session timeout
    pub fn with_session_timeout(mut self, timeout: Duration) -> Self {
        self.session_timeout = timeout;
        self
    }
    
    /// Set the server name
    pub fn with_server_name(mut self, name: String) -> Self {
        self.server_name = name;
        self
    }
    
    /// Set the contact URI
    pub fn with_contact_uri(mut self, uri: String) -> Self {
        self.contact_uri = Some(uri);
        self
    }
    
    /// Enable or disable media coordination
    pub fn with_media(mut self, enable: bool) -> Self {
        self.enable_media = enable;
        self
    }
    
    /// Validate the configuration
    pub fn validate(&self) -> Result<()> {
        // Validate bind address
        if self.bind_address.port() == 0 {
            return Err(anyhow::anyhow!("Bind address must have a valid port"));
        }
        
        // Validate max sessions
        if self.max_sessions == 0 {
            return Err(anyhow::anyhow!("Max sessions must be greater than 0"));
        }
        
        if self.max_sessions > 100_000 {
            return Err(anyhow::anyhow!("Max sessions cannot exceed 100,000"));
        }
        
        // Validate timeouts
        if self.session_timeout.as_secs() < 30 {
            return Err(anyhow::anyhow!("Session timeout must be at least 30 seconds"));
        }
        
        if self.transaction_timeout.as_secs() < 1 {
            return Err(anyhow::anyhow!("Transaction timeout must be at least 1 second"));
        }
        
        // Validate server name
        if self.server_name.is_empty() {
            return Err(anyhow::anyhow!("Server name cannot be empty"));
        }
        
        // Validate contact URI if provided
        if let Some(ref uri) = self.contact_uri {
            if uri.is_empty() {
                return Err(anyhow::anyhow!("Contact URI cannot be empty if provided"));
            }
            
            if !uri.starts_with("sip:") && !uri.starts_with("sips:") {
                return Err(anyhow::anyhow!("Contact URI must be a valid SIP URI"));
            }
        }
        
        Ok(())
    }
    
    /// Get the effective contact URI
    pub fn effective_contact_uri(&self) -> String {
        self.contact_uri.clone().unwrap_or_else(|| {
            format!("sip:{}:{}", 
                self.bind_address.ip(), 
                self.bind_address.port()
            )
        })
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_default_config() {
        let config = ServerConfig::default();
        assert!(config.validate().is_ok());
        assert_eq!(config.transport_protocol, TransportProtocol::Udp);
        assert_eq!(config.max_sessions, 1000);
        assert!(config.enable_media);
    }
    
    #[test]
    fn test_config_builder() {
        let config = ServerConfig::new("192.168.1.100:5060".parse().unwrap())
            .with_transport(TransportProtocol::Tcp)
            .with_max_sessions(500)
            .with_server_name("test-server".to_string());
            
        assert!(config.validate().is_ok());
        assert_eq!(config.transport_protocol, TransportProtocol::Tcp);
        assert_eq!(config.max_sessions, 500);
        assert_eq!(config.server_name, "test-server");
    }
    
    #[test]
    fn test_validation_errors() {
        let mut config = ServerConfig::default();
        
        // Test invalid max sessions
        config.max_sessions = 0;
        assert!(config.validate().is_err());
        
        // Test invalid timeout
        config.max_sessions = 100;
        config.session_timeout = Duration::from_secs(10);
        assert!(config.validate().is_err());
    }
} 