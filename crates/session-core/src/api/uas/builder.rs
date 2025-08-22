//! UAS Builder pattern for flexible server configuration

use std::sync::Arc;
use crate::errors::Result;
use super::{UasServer, UasConfig, UasCallHandler, CallController, AlwaysAcceptHandler, NoOpController};

/// Builder for creating UAS servers with flexible configuration
pub struct UasBuilder {
    config: UasConfig,
    handler: Option<Arc<dyn UasCallHandler>>,
    controller: Option<Arc<dyn CallController>>,
}

impl UasBuilder {
    /// Create a new builder with bind address
    pub fn new(bind_addr: impl Into<String>) -> Self {
        Self {
            config: UasConfig {
                local_addr: bind_addr.into(),
                ..Default::default()
            },
            handler: None,
            controller: None,
        }
    }
    
    /// Set server identity
    pub fn identity(mut self, identity: impl Into<String>) -> Self {
        self.config.identity = identity.into();
        self
    }
    
    /// Set user agent string
    pub fn user_agent(mut self, ua: impl Into<String>) -> Self {
        self.config.user_agent = ua.into();
        self
    }
    
    /// Set maximum concurrent calls (0 = unlimited)
    pub fn max_concurrent_calls(mut self, max: usize) -> Self {
        self.config.max_concurrent_calls = max;
        self
    }
    
    /// Enable auto-reject when at capacity
    pub fn auto_reject_on_busy(mut self, enabled: bool) -> Self {
        self.config.auto_reject_on_busy = enabled;
        self
    }
    
    /// Enable authentication
    pub fn require_authentication(mut self, realm: impl Into<String>) -> Self {
        self.config.require_authentication = true;
        self.config.auth_realm = Some(realm.into());
        self
    }
    
    /// Set call timeout in seconds
    pub fn call_timeout(mut self, seconds: u32) -> Self {
        self.config.call_timeout = seconds;
        self
    }
    
    /// Enable auto-answer for testing
    pub fn auto_answer(mut self, enabled: bool) -> Self {
        self.config.auto_answer = enabled;
        self
    }
    
    /// Enable call recording
    pub fn enable_recording(mut self, enabled: bool) -> Self {
        self.config.enable_recording = enabled;
        self
    }
    
    /// Set the call handler
    pub fn handler(mut self, handler: Arc<dyn UasCallHandler>) -> Self {
        self.handler = Some(handler);
        self
    }
    
    /// Set the call controller for advanced features
    pub fn controller(mut self, controller: Arc<dyn CallController>) -> Self {
        self.controller = Some(controller);
        self
    }
    
    /// Build the UAS server
    pub async fn build(self) -> Result<UasServer> {
        // Use defaults if not specified
        let handler = self.handler.unwrap_or_else(|| Arc::new(AlwaysAcceptHandler));
        let controller = self.controller.unwrap_or_else(|| Arc::new(NoOpController));
        
        UasServer::new_with_controller(self.config, handler, controller).await
    }
}