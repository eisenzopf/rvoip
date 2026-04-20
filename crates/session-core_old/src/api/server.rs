//! Server-oriented Session Management API
//! 
//! Provides server-specific functionality for call centers, PBXs, and other
//! server applications that need to manage multiple sessions and bridges.

use async_trait::async_trait;
use tokio::sync::mpsc;
use std::sync::Arc;
use std::time::Duration;

use crate::api::types::{SessionId, CallSession};
use crate::errors::Result;

use super::bridge::{BridgeId, BridgeInfo, BridgeEvent};

/// Server-oriented session manager with bridge capabilities
#[async_trait]
pub trait ServerSessionManager: Send + Sync {
    /// Create a bridge between two or more sessions
    async fn bridge_sessions(&self, session1: &SessionId, session2: &SessionId) -> Result<BridgeId>;
    
    /// Destroy an existing bridge
    async fn destroy_bridge(&self, bridge_id: &BridgeId) -> Result<()>;
    
    /// Get the bridge a session is part of (if any)
    async fn get_session_bridge(&self, session_id: &SessionId) -> Result<Option<BridgeId>>;
    
    /// Remove a session from a bridge
    async fn remove_session_from_bridge(&self, bridge_id: &BridgeId, session_id: &SessionId) -> Result<()>;
    
    /// List all active bridges
    async fn list_bridges(&self) -> Vec<BridgeInfo>;
    
    /// Subscribe to bridge events
    async fn subscribe_to_bridge_events(&self) -> mpsc::UnboundedReceiver<BridgeEvent>;
    
    /// Create a pre-allocated outgoing session (for agent registration)
    async fn create_outgoing_session(&self) -> Result<SessionId>;
}

/// Configuration for server-oriented session management
#[derive(Debug, Clone)]
pub struct ServerConfig {
    pub bind_address: std::net::SocketAddr,
    pub transport_protocol: TransportProtocol,
    pub max_sessions: usize,
    pub session_timeout: Duration,
    pub transaction_timeout: Duration,
    pub enable_media: bool,
    pub server_name: String,
    pub contact_uri: Option<String>,
}

impl Default for ServerConfig {
    fn default() -> Self {
        Self {
            bind_address: "0.0.0.0:5060".parse().unwrap(),
            transport_protocol: TransportProtocol::Udp,
            max_sessions: 1000,
            session_timeout: Duration::from_secs(3600), // 1 hour
            transaction_timeout: Duration::from_secs(32), // RFC 3261 Timer B
            enable_media: true,
            server_name: "RVOIP-Server/1.0".to_string(),
            contact_uri: None,
        }
    }
}

/// Transport protocol for SIP
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TransportProtocol {
    Udp,
    Tcp,
    Tls,
    Ws,
    Wss,
}

/// Factory function to create a server session manager
pub async fn create_full_server_manager(
    transaction_manager: Arc<rvoip_dialog_core::transaction::TransactionManager>,
    config: ServerConfig,
) -> Result<Arc<dyn ServerSessionManager>> {
    // TODO: Implement server-specific session manager
    // For now, return an error indicating this is not yet implemented
    Err(crate::errors::SessionError::NotImplemented { 
        feature: "Server session manager".to_string() 
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_server_config_default() {
        let config = ServerConfig::default();
        assert_eq!(config.max_sessions, 1000);
        assert_eq!(config.transport_protocol, TransportProtocol::Udp);
    }
}
