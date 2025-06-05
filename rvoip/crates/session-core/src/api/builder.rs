//! SessionManager Builder Pattern
//!
//! Provides a simple, fluent API for configuring and creating SessionManager instances.

use std::sync::Arc;
use crate::errors::Result;
use crate::manager::SessionManager;
use crate::api::handlers::CallHandler;

/// Builder for creating SessionManager instances with sensible defaults
#[derive(Debug)]
pub struct SessionManagerBuilder {
    sip_port: Option<u16>,
    media_port_start: Option<u16>,
    media_port_end: Option<u16>,
    handler: Option<Arc<dyn CallHandler>>,
    p2p_mode: bool,
}

impl Default for SessionManagerBuilder {
    fn default() -> Self {
        Self::new()
    }
}

impl SessionManagerBuilder {
    /// Create a new builder with default settings
    pub fn new() -> Self {
        Self {
            sip_port: None,
            media_port_start: None,
            media_port_end: None,
            handler: None,
            p2p_mode: false,
        }
    }

    /// Set the SIP listening port (default: 5060)
    pub fn with_sip_port(mut self, port: u16) -> Self {
        self.sip_port = Some(port);
        self
    }

    /// Set the range of ports for media (RTP/RTCP)
    pub fn with_media_ports(mut self, start: u16, end: u16) -> Self {
        self.media_port_start = Some(start);
        self.media_port_end = Some(end);
        self
    }

    /// Set the call handler for managing incoming calls
    pub fn with_handler(mut self, handler: Arc<dyn CallHandler>) -> Self {
        self.handler = Some(handler);
        self
    }

    /// Enable peer-to-peer mode (no server required)
    pub fn p2p_mode(mut self) -> Self {
        self.p2p_mode = true;
        self
    }

    /// Build the SessionManager with the configured settings
    pub async fn build(self) -> Result<Arc<SessionManager>> {
        let config = SessionManagerConfig {
            sip_port: self.sip_port.unwrap_or(5060),
            media_port_start: self.media_port_start.unwrap_or(10000),
            media_port_end: self.media_port_end.unwrap_or(20000),
            p2p_mode: self.p2p_mode,
        };

        SessionManager::new(config, self.handler).await
    }
}

/// Configuration for SessionManager
#[derive(Debug, Clone)]
pub struct SessionManagerConfig {
    pub sip_port: u16,
    pub media_port_start: u16,
    pub media_port_end: u16,
    pub p2p_mode: bool,
}

impl Default for SessionManagerConfig {
    fn default() -> Self {
        Self {
            sip_port: 5060,
            media_port_start: 10000,
            media_port_end: 20000,
            p2p_mode: false,
        }
    }
} 