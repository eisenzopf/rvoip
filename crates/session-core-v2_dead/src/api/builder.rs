//! Simplified Session Builder
//!
//! Just builds the UnifiedCoordinator with configuration.
//! No complex setup - the state table handles everything.

use crate::api::unified::{UnifiedCoordinator, Config};
use crate::errors::Result;
use std::net::{IpAddr, SocketAddr};
use std::sync::Arc;

/// Builder for creating a UnifiedCoordinator
pub struct SessionBuilder {
    config: Config,
}

impl SessionBuilder {
    /// Create a new builder with default configuration
    pub fn new() -> Self {
        Self {
            config: Config::default(),
        }
    }
    
    /// Set the SIP port
    pub fn with_sip_port(mut self, port: u16) -> Self {
        self.config.sip_port = port;
        self.config.bind_addr.set_port(port);
        self
    }
    
    /// Set the media port range
    pub fn with_media_ports(mut self, start: u16, end: u16) -> Self {
        self.config.media_port_start = start;
        self.config.media_port_end = end;
        self
    }
    
    /// Set the local IP address
    pub fn with_local_ip(mut self, ip: IpAddr) -> Self {
        self.config.local_ip = ip;
        self.config.bind_addr.set_ip(ip);
        self
    }
    
    /// Set the bind address
    pub fn with_bind_addr(mut self, addr: SocketAddr) -> Self {
        self.config.bind_addr = addr;
        self.config.local_ip = addr.ip();
        self.config.sip_port = addr.port();
        self
    }
    
    /// Build the UnifiedCoordinator
    pub async fn build(self) -> Result<Arc<UnifiedCoordinator>> {
        UnifiedCoordinator::new(self.config).await
    }
}

impl Default for SessionBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_builder_configuration() {
        let builder = SessionBuilder::new()
            .with_sip_port(5061)
            .with_media_ports(20000, 30000)
            .with_local_ip("192.168.1.100".parse().unwrap());
        
        assert_eq!(builder.config.sip_port, 5061);
        assert_eq!(builder.config.media_port_start, 20000);
        assert_eq!(builder.config.media_port_end, 30000);
        assert_eq!(builder.config.local_ip.to_string(), "192.168.1.100");
    }
}