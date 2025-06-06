//! Dialog Config Converter (parallel to MediaConfigConverter)
//!
//! Handles conversion between session-level configuration and 
//! dialog-core SIP configuration.

use std::net::SocketAddr;
use crate::api::builder::SessionManagerConfig;
use crate::dialog::{DialogError, DialogResult};

/// Dialog configuration converter for session-dialog coordination
/// (parallel to MediaConfigConverter)
pub struct DialogConfigConverter {
    session_config: SessionManagerConfig,
}

impl DialogConfigConverter {
    /// Create a new dialog config converter
    pub fn new(session_config: SessionManagerConfig) -> Self {
        Self { session_config }
    }
    
    /// Get access to the session configuration
    pub fn session_config(&self) -> &SessionManagerConfig {
        &self.session_config
    }
    
    /// Convert session config to dialog-core configuration
    pub fn to_dialog_config(&self) -> DialogResult<rvoip_dialog_core::config::DialogManagerConfig> {
        // Parse bind address from string
        let bind_addr = format!("{}:{}", self.session_config.sip_bind_address, self.session_config.sip_port);
        let local_address: SocketAddr = bind_addr.parse()
            .map_err(|e| DialogError::Configuration {
                message: format!("Invalid bind address: {}", e),
            })?;
        
        let dialog_config = rvoip_dialog_core::config::DialogManagerConfig::hybrid(local_address)
            .build();
            
        Ok(dialog_config)
    }
    
    /// Extract SIP headers from session configuration
    pub fn extract_sip_headers(&self) -> std::collections::HashMap<String, String> {
        let mut headers = std::collections::HashMap::new();
        
        // Add any session-specific SIP headers
        headers.insert("User-Agent".to_string(), "rvoip-session-core".to_string());
        
        headers
    }
    
    /// Get default dialog timeouts for session operations
    pub fn get_default_timeouts(&self) -> DialogResult<(std::time::Duration, std::time::Duration)> {
        // Return (invite_timeout, bye_timeout)
        Ok((
            std::time::Duration::from_secs(30),
            std::time::Duration::from_secs(10),
        ))
    }
    
    /// Validate dialog configuration compatibility with session config
    pub fn validate_compatibility(&self) -> DialogResult<()> {
        // Ensure the SIP port is valid
        if self.session_config.sip_port == 0 {
            return Err(DialogError::Configuration {
                message: "SIP port cannot be 0".to_string(),
            });
        }
        
        // Ensure bind address is valid
        let bind_addr: std::net::SocketAddr = format!("{}:{}", self.session_config.sip_bind_address, self.session_config.sip_port)
            .parse()
            .map_err(|e| DialogError::Configuration {
                message: format!("Invalid bind address format: {}", e),
            })?;
            
        if bind_addr.ip().is_unspecified() {
            return Err(DialogError::Configuration {
                message: "Bind address cannot be unspecified".to_string(),
            });
        }
        
        Ok(())
    }
}

impl std::fmt::Debug for DialogConfigConverter {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DialogConfigConverter")
            .field("sip_port", &self.session_config.sip_port)
            .field("bind_address", &self.session_config.sip_bind_address)
            .finish()
    }
} 