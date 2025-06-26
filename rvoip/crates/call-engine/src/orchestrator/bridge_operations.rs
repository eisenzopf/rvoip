//! Bridge operations for the call center
//!
//! This module handles actual bridge operations via session-core including
//! conferences, transfers, and bridge monitoring.

use std::sync::Arc;
use tracing::{info, warn};
use tokio::sync::mpsc;
use rvoip_session_core::{SessionId, BridgeId, BridgeInfo, BridgeEvent};

use crate::agent::AgentId;
use crate::error::{CallCenterError, Result as CallCenterResult};
use super::core::CallCenterEngine;

impl CallCenterEngine {
    /// Create a conference with multiple participants
    pub async fn create_conference(&self, session_ids: &[SessionId]) -> CallCenterResult<BridgeId> {
        info!("🎤 Creating conference with {} participants", session_ids.len());
        
        // **REAL**: Create bridge using session-core API
        let bridge_id = self.session_coordinator.as_ref().unwrap()
            .create_bridge()
            .await
            .map_err(|e| CallCenterError::orchestration(&format!("Failed to create conference bridge: {}", e)))?;
        
        // **REAL**: Add all sessions to the bridge
        for session_id in session_ids {
            self.session_coordinator.as_ref().unwrap()
                .add_session_to_bridge(&bridge_id, session_id)
                .await
                .map_err(|e| CallCenterError::orchestration(&format!("Failed to add session {} to conference: {}", session_id, e)))?;
        }
        
        info!("✅ Created conference bridge: {}", bridge_id);
        Ok(bridge_id)
    }
    
    /// Transfer call from one agent to another
    pub async fn transfer_call(
        &self,
        customer_session: SessionId,
        from_agent: AgentId,
        to_agent: AgentId,
    ) -> CallCenterResult<BridgeId> {
        info!("🔄 Transferring call from agent {} to agent {}", from_agent, to_agent);
        
        // Check if agent is available in database
        let to_agent_available = if let Some(db_manager) = &self.db_manager {
            match db_manager.get_agent(&to_agent.0).await {
                Ok(Some(agent)) => matches!(agent.status, crate::database::DbAgentStatus::Available),
                _ => false,
            }
        } else {
            false
        };
        
        if !to_agent_available {
            return Err(CallCenterError::orchestration(&format!("Agent {} not available", to_agent)));
        }
        
        // TODO: Create a new session for the to_agent and establish the transfer
        // For now, return an error as we need the session ID
        return Err(CallCenterError::orchestration("Call transfer not yet implemented without agent session tracking"));
        
        // The code below is unreachable but kept for future implementation reference:
        // 
        // // Get current bridge if any
        // if let Ok(Some(current_bridge)) = self.session_coordinator.as_ref().unwrap()
        //     .get_session_bridge(&customer_session).await {
        //     // **REAL**: Remove customer from current bridge
        //     if let Err(e) = self.session_coordinator.as_ref().unwrap()
        //         .remove_session_from_bridge(&current_bridge, &customer_session).await {
        //         warn!("Failed to remove customer from current bridge: {}", e);
        //     }
        // }
        // 
        // // **REAL**: Create new bridge with customer and new agent
        // let new_bridge = self.session_coordinator.as_ref().unwrap()
        //     .bridge_sessions(&customer_session, &to_agent_session)
        //     .await
        //     .map_err(|e| CallCenterError::orchestration(&format!("Failed to create transfer bridge: {}", e)))?;
        // 
        // info!("✅ Call transferred successfully to bridge: {}", new_bridge);
        // Ok(new_bridge)
    }
    
    /// Get real-time bridge information for monitoring
    pub async fn get_bridge_info(&self, bridge_id: &BridgeId) -> CallCenterResult<BridgeInfo> {
        self.session_coordinator.as_ref().unwrap()
            .get_bridge_info(bridge_id)
            .await
            .map_err(|e| CallCenterError::orchestration(&format!("Failed to get bridge info: {}", e)))?
            .ok_or_else(|| CallCenterError::not_found(format!("Bridge not found: {}", bridge_id)))
    }
    
    /// List all active bridges for dashboard
    pub async fn list_active_bridges(&self) -> Vec<BridgeInfo> {
        self.session_coordinator.as_ref().unwrap().list_bridges().await
    }
    
    /// Subscribe to bridge events for real-time monitoring
    pub async fn start_bridge_monitoring(&mut self) -> CallCenterResult<()> {
        info!("👁️ Starting bridge event monitoring");
        
        // **REAL**: Subscribe to session-core bridge events
        let event_receiver = self.session_coordinator.as_ref().unwrap()
            .subscribe_to_bridge_events().await;
        self.bridge_events = Some(event_receiver);
        
        // Process events in background task
        if let Some(mut receiver) = self.bridge_events.take() {
            let engine = Arc::new(self.clone());
            tokio::spawn(async move {
                while let Some(event) = receiver.recv().await {
                    engine.handle_bridge_event(event).await;
                }
            });
        }
        
        Ok(())
    }
    
    /// Handle bridge events for monitoring and metrics
    pub(super) async fn handle_bridge_event(&self, event: BridgeEvent) {
        match event {
            BridgeEvent::ParticipantAdded { bridge_id, session_id } => {
                info!("➕ Session {} added to bridge {}", session_id, bridge_id);
            },
            BridgeEvent::ParticipantRemoved { bridge_id, session_id, reason } => {
                info!("➖ Session {} removed from bridge {}: {}", session_id, bridge_id, reason);
            },
            BridgeEvent::BridgeDestroyed { bridge_id } => {
                info!("🗑️ Bridge destroyed: {}", bridge_id);
            },
        }
    }
} 