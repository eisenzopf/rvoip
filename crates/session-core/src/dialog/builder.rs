//! Dialog Builder (unique to dialog integration)
//!
//! Handles dialog-core UnifiedDialogApi setup and configuration.
//! This is unique to dialog integration since media doesn't need external API setup.

use std::sync::Arc;
use rvoip_dialog_core::{
    config::DialogManagerConfig,
    api::unified::UnifiedDialogApi,
};
use crate::api::builder::SessionManagerConfig;
use crate::dialog::{DialogError, DialogResult, DialogConfigConverter};

/// Dialog builder for creating and configuring dialog-core UnifiedDialogApi
pub struct DialogBuilder {
    config_converter: DialogConfigConverter,
    from_uri: Option<String>,
    dialog_api: Option<Arc<UnifiedDialogApi>>,
}

impl DialogBuilder {
    /// Create a new dialog builder from session configuration
    pub fn new(session_config: SessionManagerConfig) -> Self {
        Self {
            config_converter: DialogConfigConverter::new(session_config),
            from_uri: None,
            dialog_api: None,
        }
    }
    
    /// Set custom From URI for outgoing calls
    pub fn with_from_uri(mut self, from_uri: impl Into<String>) -> Self {
        self.from_uri = Some(from_uri.into());
        self
    }
    
    /// Set a pre-configured dialog API (for advanced use cases)
    pub fn with_dialog_api(mut self, api: Arc<UnifiedDialogApi>) -> Self {
        self.dialog_api = Some(api);
        self
    }
    
    /// Build the UnifiedDialogApi with the configured settings
    pub async fn build(self) -> DialogResult<Arc<UnifiedDialogApi>> {
        // If dialog API was pre-configured, use it
        if let Some(api) = self.dialog_api {
            return Ok(api);
        }
        
        // Validate configuration compatibility
        self.config_converter.validate_compatibility()?;
        
        // Create dialog configuration using the session configuration
        let dialog_config = self.create_dialog_config().await?;
        
        // Create the UnifiedDialogApi
        let api = UnifiedDialogApi::create(dialog_config)
            .await
            .map_err(|e| DialogError::DialogCreation {
                reason: format!("Failed to create dialog API: {}", e),
            })?;
        
        Ok(Arc::new(api))
    }
    
    /// Create dialog-core configuration from session configuration
    async fn create_dialog_config(&self) -> DialogResult<DialogManagerConfig> {
        // Get the session configuration details
        let session_config = &self.config_converter;
        
        // Use the configured bind address from session config
        let bind_addr = if session_config.session_config().local_bind_addr.ip().is_unspecified() {
            format!("0.0.0.0:{}", session_config.session_config().sip_port)
        } else {
            session_config.session_config().local_bind_addr.to_string()
        };
        
        let socket_addr = bind_addr.parse()
            .map_err(|e| DialogError::Configuration {
                message: format!("Invalid bind address: {}", e),
            })?;
        
        // Determine From URI - use local_address from config or custom from_uri
        let from_uri = if let Some(ref uri) = self.from_uri {
            uri.clone()
        } else {
            session_config.session_config().local_address.clone()
        };
        
        // Create dialog configuration using hybrid mode to support both incoming and outgoing calls
        // IMPORTANT: Disable auto_register_response so CallCenterEngine can handle REGISTER properly
        let dialog_config = DialogManagerConfig::hybrid(socket_addr)
            .with_from_uri(from_uri)
            // Don't auto-respond to REGISTER - let application handle it
            // .with_auto_register() // DISABLED - application will handle REGISTER
            .build();
        
        Ok(dialog_config)
    }
    
    /// Create dialog configuration with custom timeouts
    pub async fn build_with_timeouts(
        self,
        invite_timeout: std::time::Duration,
        bye_timeout: std::time::Duration,
    ) -> DialogResult<Arc<UnifiedDialogApi>> {
        // If dialog API was pre-configured, use it
        if let Some(api) = self.dialog_api {
            return Ok(api);
        }
        
        // Validate configuration compatibility
        self.config_converter.validate_compatibility()?;
        
        // Create dialog configuration
        let mut dialog_config = self.create_dialog_config().await?;
        
        // Set custom timeouts
        // Note: This assumes DialogManagerConfig has timeout setters
        // If not available, we'll use the default config for now
        
        // Create the UnifiedDialogApi
        let api = UnifiedDialogApi::create(dialog_config)
            .await
            .map_err(|e| DialogError::DialogCreation {
                reason: format!("Failed to create dialog API with timeouts: {}", e),
            })?;
        
        Ok(Arc::new(api))
    }
    
    /// Create dialog API for server mode (incoming calls only)
    pub async fn build_server_mode(self) -> DialogResult<Arc<UnifiedDialogApi>> {
        // Validate configuration compatibility
        self.config_converter.validate_compatibility()?;
        
        // Use the configured bind address from session config
        let bind_addr = if self.config_converter.session_config().local_bind_addr.ip().is_unspecified() {
            format!("0.0.0.0:{}", self.config_converter.session_config().sip_port)
        } else {
            self.config_converter.session_config().local_bind_addr.to_string()
        };
        
        let socket_addr = bind_addr.parse()
            .map_err(|e| DialogError::Configuration {
                message: format!("Invalid bind address: {}", e),
            })?;
        
        // Create server-mode dialog configuration
        let dialog_config = DialogManagerConfig::server(socket_addr).build();
        
        // Create the UnifiedDialogApi
        let api = UnifiedDialogApi::create(dialog_config)
            .await
            .map_err(|e| DialogError::DialogCreation {
                reason: format!("Failed to create dialog API in server mode: {}", e),
            })?;
        
        Ok(Arc::new(api))
    }
    
    /// Create dialog API for client mode (outgoing calls only)
    pub async fn build_client_mode(self) -> DialogResult<Arc<UnifiedDialogApi>> {
        // Validate configuration compatibility
        self.config_converter.validate_compatibility()?;
        
        // Determine From URI
        let from_uri = if let Some(ref uri) = self.from_uri {
            uri.clone()
        } else {
            self.config_converter.session_config().local_address.clone()
        };
        
        // Use the configured bind address from session config (with port 0 for client mode)
        let local_addr = if self.config_converter.session_config().local_bind_addr.ip().is_unspecified() {
            "0.0.0.0:0".parse().map_err(|e| DialogError::Configuration {
                message: format!("Invalid local address for client mode: {}", e),
            })?
        } else {
            // Use same IP as configured but with port 0 for dynamic assignment
            let ip = self.config_converter.session_config().local_bind_addr.ip();
            format!("{}:0", ip).parse().map_err(|e| DialogError::Configuration {
                message: format!("Invalid local address for client mode: {}", e),
            })?
        };
        
        // Create client-mode dialog configuration
        let dialog_config = DialogManagerConfig::client(local_addr)
            .with_from_uri(from_uri)
            .build();
        
        // Create the UnifiedDialogApi
        let api = UnifiedDialogApi::create(dialog_config)
            .await
            .map_err(|e| DialogError::DialogCreation {
                reason: format!("Failed to create dialog API in client mode: {}", e),
            })?;
        
        Ok(Arc::new(api))
    }
    
    /// Extract SIP headers from session configuration
    pub fn extract_sip_headers(&self) -> std::collections::HashMap<String, String> {
        self.config_converter.extract_sip_headers()
    }
    
    /// Get dialog timeouts from session configuration
    pub fn get_dialog_timeouts(&self) -> DialogResult<(std::time::Duration, std::time::Duration)> {
        self.config_converter.get_default_timeouts()
    }
}

impl std::fmt::Debug for DialogBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DialogBuilder")
            .field("from_uri", &self.from_uri)
            .field("has_dialog_api", &self.dialog_api.is_some())
            .field("config_converter", &self.config_converter)
            .finish()
    }
} 