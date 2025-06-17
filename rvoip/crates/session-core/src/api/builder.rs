//! Session Manager Builder API
//!
//! Provides a fluent builder interface for creating and configuring
//! the SessionManager with all necessary components.

use std::sync::Arc;
use crate::api::handlers::CallHandler;
use crate::coordinator::SessionCoordinator;
use crate::errors::Result;

/// Configuration for the SessionManager
#[derive(Debug, Clone)]
pub struct SessionManagerConfig {
    /// SIP listening port
    pub sip_port: u16,
    
    /// Local SIP address (e.g., "user@domain")
    pub local_address: String,
    
    /// Media port range start
    pub media_port_start: u16,
    
    /// Media port range end
    pub media_port_end: u16,
    
    /// Enable STUN for NAT traversal
    pub enable_stun: bool,
    
    /// STUN server address
    pub stun_server: Option<String>,
}

impl Default for SessionManagerConfig {
    fn default() -> Self {
        Self {
            sip_port: 5060,
            local_address: "sip:user@localhost".to_string(),
            media_port_start: 10000,
            media_port_end: 20000,
            enable_stun: false,
            stun_server: None,
        }
    }
}

/// Builder for creating a configured SessionManager
pub struct SessionManagerBuilder {
    config: SessionManagerConfig,
    handler: Option<Arc<dyn CallHandler>>,
}

impl SessionManagerBuilder {
    /// Create a new builder with default configuration
    pub fn new() -> Self {
        Self {
            config: SessionManagerConfig::default(),
            handler: None,
        }
    }
    
    /// Set the SIP listening port
    pub fn with_sip_port(mut self, port: u16) -> Self {
        self.config.sip_port = port;
        self
    }
    
    /// Set the local SIP address
    pub fn with_local_address(mut self, address: impl Into<String>) -> Self {
        self.config.local_address = address.into();
        self
    }
    
    /// Set the media port range
    pub fn with_media_ports(mut self, start: u16, end: u16) -> Self {
        self.config.media_port_start = start;
        self.config.media_port_end = end;
        self
    }
    
    /// Enable STUN with the specified server
    pub fn with_stun(mut self, server: impl Into<String>) -> Self {
        self.config.enable_stun = true;
        self.config.stun_server = Some(server.into());
        self
    }
    
    /// Set the call event handler
    pub fn with_handler(mut self, handler: Arc<dyn CallHandler>) -> Self {
        self.handler = Some(handler);
        self
    }
    
    /// Build and initialize the SessionManager
    pub async fn build(self) -> Result<Arc<SessionCoordinator>> {
        // Create the top-level coordinator with all subsystems
        let coordinator = SessionCoordinator::new(
            self.config,
            self.handler,
        ).await?;
        
        // Start all subsystems
        coordinator.start().await?;
        
        Ok(coordinator)
    }
}

impl Default for SessionManagerBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_builder_defaults() {
        let builder = SessionManagerBuilder::new();
        assert_eq!(builder.config.sip_port, 5060);
        assert_eq!(builder.config.media_port_start, 10000);
        assert_eq!(builder.config.media_port_end, 20000);
    }
    
    #[test]
    fn test_builder_configuration() {
        let builder = SessionManagerBuilder::new()
            .with_sip_port(5061)
            .with_local_address("alice@example.com")
            .with_media_ports(30000, 40000)
            .with_stun("stun.example.com:3478");
            
        assert_eq!(builder.config.sip_port, 5061);
        assert_eq!(builder.config.local_address, "alice@example.com");
        assert_eq!(builder.config.media_port_start, 30000);
        assert_eq!(builder.config.media_port_end, 40000);
        assert!(builder.config.enable_stun);
        assert_eq!(builder.config.stun_server, Some("stun.example.com:3478".to_string()));
    }
} 