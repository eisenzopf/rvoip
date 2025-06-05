//! SessionManager Builder Pattern
//!
//! Provides a simple, fluent API for configuring and creating SessionManager instances.

use std::sync::Arc;
use crate::errors::Result;
use crate::manager::SessionManager;
use crate::api::handlers::CallHandler;

// Dialog-core integration - using UnifiedDialogApi
use rvoip_dialog_core::{config::DialogManagerConfig, api::unified::UnifiedDialogApi};

/// Builder for creating SessionManager instances with sensible defaults
pub struct SessionManagerBuilder {
    sip_port: Option<u16>,
    sip_bind_address: Option<String>,
    from_uri: Option<String>,
    media_port_start: Option<u16>,
    media_port_end: Option<u16>,
    handler: Option<Arc<dyn CallHandler>>,
    p2p_mode: bool,
    dialog_api: Option<Arc<UnifiedDialogApi>>,
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
            .field("dialog_api", &self.dialog_api.is_some())
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
            dialog_api: None,
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

    /// Set a pre-configured dialog API (for advanced use cases)
    pub fn with_dialog_api(mut self, api: Arc<UnifiedDialogApi>) -> Self {
        self.dialog_api = Some(api);
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

        // Create dialog API if not provided
        let dialog_api = if let Some(api) = self.dialog_api {
            api
        } else {
            // Create dialog configuration using the proper unified API
            let bind_addr = format!("{}:{}", sip_bind_address, sip_port);
            let socket_addr = bind_addr.parse().map_err(|e| {
                crate::errors::SessionError::internal(&format!("Invalid bind address: {}", e))
            })?;
            
            // Use hybrid mode to support both incoming and outgoing calls
            let dialog_config = if let Some(ref from_uri) = self.from_uri {
                DialogManagerConfig::hybrid(socket_addr)
                    .with_from_uri(from_uri.clone())
                    .build()
            } else {
                DialogManagerConfig::hybrid(socket_addr)
                    .with_from_uri(format!("sip:user@{}", sip_bind_address))
                    .build()
            };
            
            // Use UnifiedDialogApi::create() which handles all dependency injection properly
            let api = UnifiedDialogApi::create(dialog_config).await
                .map_err(|e| crate::errors::SessionError::internal(&format!("Failed to create dialog API: {}", e)))?;
            
            Arc::new(api)
        };

        SessionManager::new(config, self.handler, dialog_api).await
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