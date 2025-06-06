//! SessionManager Builder Pattern
//!
//! Provides a simple, fluent API for configuring and creating SessionManager instances.
//! Dialog integration is now handled internally by SessionManager.

use std::sync::Arc;
use crate::errors::Result;
use crate::manager::SessionManager;
use crate::api::handlers::CallHandler;

/// Builder for creating SessionManager instances with sensible defaults
pub struct SessionManagerBuilder {
    sip_port: Option<u16>,
    sip_bind_address: Option<String>,
    from_uri: Option<String>,
    media_port_start: Option<u16>,
    media_port_end: Option<u16>,
    handler: Option<Arc<dyn CallHandler>>,
    p2p_mode: bool,
}

impl std::fmt::Debug for SessionManagerBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionManagerBuilder")
            .field("sip_port", &self.sip_port)
            .field("sip_bind_address", &self.sip_bind_address)
            .field("from_uri", &self.from_uri)
            .field("media_port_start", &self.media_port_start)
            .field("media_port_end", &self.media_port_end)
            .field("handler", &self.handler.is_some())
            .field("p2p_mode", &self.p2p_mode)
            .finish()
    }
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
            sip_bind_address: None,
            from_uri: None,
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

    /// Set the SIP bind address (default: "0.0.0.0")
    pub fn with_sip_bind_address(mut self, address: impl Into<String>) -> Self {
        self.sip_bind_address = Some(address.into());
        self
    }

    /// Set the From URI for outgoing calls
    pub fn with_from_uri(mut self, uri: impl Into<String>) -> Self {
        self.from_uri = Some(uri.into());
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
        let sip_port = self.sip_port.unwrap_or(5060);
        let sip_bind_address = self.sip_bind_address.unwrap_or_else(|| "0.0.0.0".to_string());
        
        let config = SessionManagerConfig {
            sip_port,
            sip_bind_address: sip_bind_address.clone(),
            from_uri: self.from_uri.clone(),
            media_port_start: self.media_port_start.unwrap_or(10000),
            media_port_end: self.media_port_end.unwrap_or(20000),
            p2p_mode: self.p2p_mode,
        };

        // SessionManager now handles dialog integration internally (high-level abstraction)
        SessionManager::new(config, self.handler).await
    }
}

/// Configuration for SessionManager
#[derive(Debug, Clone)]
pub struct SessionManagerConfig {
    pub sip_port: u16,
    pub sip_bind_address: String,
    pub from_uri: Option<String>,
    pub media_port_start: u16,
    pub media_port_end: u16,
    pub p2p_mode: bool,
}

impl Default for SessionManagerConfig {
    fn default() -> Self {
        Self {
            sip_port: 5060,
            sip_bind_address: "0.0.0.0".to_string(),
            from_uri: None,
            media_port_start: 10000,
            media_port_end: 20000,
            p2p_mode: false,
        }
    }
} 