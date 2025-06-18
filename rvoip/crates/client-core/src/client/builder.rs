//! Client builder for creating SIP clients

use std::sync::Arc;
use crate::{ClientConfig, ClientResult, client::ClientManager};

/// Builder for creating a SIP client
pub struct ClientBuilder {
    config: ClientConfig,
}

impl ClientBuilder {
    /// Create a new client builder
    pub fn new() -> Self {
        Self {
            config: ClientConfig::default(),
        }
    }
    
    /// Set the local SIP address
    pub fn local_address(mut self, addr: std::net::SocketAddr) -> Self {
        self.config.local_sip_addr = addr;
        self
    }
    
    /// Set the local media address
    pub fn media_address(mut self, addr: std::net::SocketAddr) -> Self {
        self.config.local_media_addr = addr;
        self
    }
    
    /// Set the user agent string
    pub fn user_agent(mut self, user_agent: impl Into<String>) -> Self {
        self.config.user_agent = user_agent.into();
        self
    }
    
    /// Set the SIP domain
    pub fn domain(mut self, domain: impl Into<String>) -> Self {
        self.config.domain = Some(domain.into());
        self
    }
    
    /// Build the client
    pub async fn build(self) -> ClientResult<Arc<ClientManager>> {
        ClientManager::new(self.config).await
    }
}

impl Default for ClientBuilder {
    fn default() -> Self {
        Self::new()
    }
} 