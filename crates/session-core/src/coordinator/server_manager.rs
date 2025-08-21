//! ServerSessionManager trait implementation for SessionCoordinator

use std::sync::Arc;
use async_trait::async_trait;
use tokio::sync::mpsc;
use crate::api::{
    types::SessionId,
    server::ServerSessionManager,
    bridge::{BridgeId, BridgeInfo, BridgeEvent},
};
use crate::errors::{Result, SessionError};
use super::SessionCoordinator;

#[async_trait]
impl ServerSessionManager for Arc<SessionCoordinator> {
    /// Create a bridge between two or more sessions
    async fn bridge_sessions(&self, session1: &SessionId, session2: &SessionId) -> Result<BridgeId> {
        // Delegate to existing SessionCoordinator bridge_sessions implementation
        SessionCoordinator::bridge_sessions(self, session1, session2).await
    }
    
    /// Destroy an existing bridge
    async fn destroy_bridge(&self, bridge_id: &BridgeId) -> Result<()> {
        // Delegate to existing SessionCoordinator destroy_bridge implementation
        SessionCoordinator::destroy_bridge(self, bridge_id).await
    }
    
    /// Get the bridge a session is part of (if any)
    async fn get_session_bridge(&self, session_id: &SessionId) -> Result<Option<BridgeId>> {
        // Delegate to existing SessionCoordinator get_session_bridge implementation
        SessionCoordinator::get_session_bridge(self, session_id).await
    }
    
    /// Remove a session from a bridge
    async fn remove_session_from_bridge(&self, bridge_id: &BridgeId, session_id: &SessionId) -> Result<()> {
        // Delegate to existing SessionCoordinator remove_session_from_bridge implementation
        SessionCoordinator::remove_session_from_bridge(self, bridge_id, session_id).await
    }
    
    /// List all active bridges
    async fn list_bridges(&self) -> Vec<BridgeInfo> {
        // Delegate to existing SessionCoordinator list_bridges implementation
        SessionCoordinator::list_bridges(self).await
    }
    
    /// Subscribe to bridge events
    async fn subscribe_to_bridge_events(&self) -> mpsc::UnboundedReceiver<BridgeEvent> {
        // Delegate to existing SessionCoordinator subscribe_to_bridge_events implementation
        SessionCoordinator::subscribe_to_bridge_events(self).await
    }
    
    /// Create a pre-allocated outgoing session (for agent registration)
    async fn create_outgoing_session(&self) -> Result<SessionId> {
        // Use the general session creation API without actually sending INVITE
        // This is useful for pre-allocating sessions that agents can use later
        let session_id = SessionId::new();
        
        // Register the session in our internal registry without creating a dialog yet
        let call_session = crate::api::types::CallSession {
            id: session_id.clone(),
            from: "sip:system@internal".to_string(), // Placeholder - will be updated when used
            to: "sip:pending@internal".to_string(),   // Placeholder - will be updated when used
            state: crate::api::types::CallState::Initiating, // Special state for pre-allocated sessions
            started_at: Some(std::time::Instant::now()),
            sip_call_id: None,
        };
        
        // Create internal session from call session
        let session = crate::session::Session::from_call_session(call_session);
        
        // Register session without creating dialog yet
        self.registry.register_session(session).await?;
        
        Ok(session_id)
    }
}
