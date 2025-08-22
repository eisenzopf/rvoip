//! UAC Builder pattern for flexible configuration

use std::sync::Arc;
use crate::errors::Result;
use super::{UacClient, UacConfig, UacEventHandler, NoOpEventHandler};

/// Builder for creating UAC clients with flexible configuration
pub struct UacBuilder {
    config: UacConfig,
    event_handler: Option<Arc<dyn UacEventHandler>>,
}

impl UacBuilder {
    /// Create a new builder with identity
    pub fn new(identity: impl Into<String>) -> Self {
        Self {
            config: UacConfig {
                identity: identity.into(),
                ..Default::default()
            },
            event_handler: None,
        }
    }
    
    /// Set the SIP server address
    pub fn server(mut self, addr: impl Into<String>) -> Self {
        self.config.server_addr = addr.into();
        self
    }
    
    /// Set the local bind address
    pub fn local_addr(mut self, addr: impl Into<String>) -> Self {
        self.config.local_addr = addr.into();
        self
    }
    
    /// Set authentication credentials
    pub fn credentials(mut self, username: impl Into<String>, password: impl Into<String>) -> Self {
        self.config.username = Some(username.into());
        self.config.password = Some(password.into());
        self
    }
    
    /// Set the user agent string
    pub fn user_agent(mut self, ua: impl Into<String>) -> Self {
        self.config.user_agent = ua.into();
        self
    }
    
    /// Enable auto-answer for testing
    pub fn auto_answer(mut self, enabled: bool) -> Self {
        self.config.auto_answer = enabled;
        self
    }
    
    /// Set registration expiry in seconds
    pub fn registration_expiry(mut self, seconds: u32) -> Self {
        self.config.registration_expiry = seconds;
        self
    }
    
    /// Set default call timeout in seconds
    pub fn call_timeout(mut self, seconds: u32) -> Self {
        self.config.call_timeout = seconds;
        self
    }
    
    /// Set event handler
    pub fn event_handler(mut self, handler: Arc<dyn UacEventHandler>) -> Self {
        self.event_handler = Some(handler);
        self
    }
    
    /// Build the UAC client
    pub async fn build(self) -> Result<UacClient> {
        // Validate configuration
        if self.config.identity.is_empty() {
            return Err(crate::errors::SessionError::ConfigError(
                "Identity is required".to_string()
            ));
        }
        
        if self.config.server_addr.is_empty() {
            return Err(crate::errors::SessionError::ConfigError(
                "Server address is required".to_string()
            ));
        }
        
        let event_handler = self.event_handler.unwrap_or_else(|| Arc::new(NoOpEventHandler));
        
        UacClient::new_with_handler(self.config, event_handler).await
    }
}