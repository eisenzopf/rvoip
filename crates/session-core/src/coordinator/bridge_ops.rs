//! Bridge management operations for SessionCoordinator

use std::time::Instant;
use tokio::sync::mpsc;
use crate::api::{
    types::SessionId,
    bridge::{BridgeId, BridgeInfo, BridgeEvent},
};
use crate::errors::{Result, SessionError};
use crate::conference::{ConferenceId, ConferenceConfig, ConferenceApi};
use super::SessionCoordinator;

impl SessionCoordinator {
    /// Create a bridge between two sessions
    pub async fn bridge_sessions(
        &self,
        session1: &SessionId,
        session2: &SessionId,
    ) -> Result<BridgeId> {
        // Use conference module to create a 2-party conference
        let bridge_id = BridgeId::new();
        let conf_id = ConferenceId::from_name(&bridge_id.0);
        
        // Create conference config for bridge (2-party conference)
        let config = ConferenceConfig {
            name: bridge_id.0.clone(),
            max_participants: 2,
            audio_mixing_enabled: true,
            audio_sample_rate: 8000,  // Standard telephony rate
            audio_channels: 1,        // Mono
            rtp_port_range: Some((10000, 20000)),
            timeout: None,            // No timeout for bridges
        };
        
        // Create the conference
        self.conference_manager.create_named_conference(conf_id.clone(), config).await
            .map_err(|e| SessionError::internal(&format!("Failed to create bridge: {}", e)))?;
        
        // Join both sessions to the conference
        self.conference_manager.join_conference(&conf_id, session1).await
            .map_err(|e| SessionError::internal(&format!("Failed to add session1 to bridge: {}", e)))?;
            
        self.conference_manager.join_conference(&conf_id, session2).await
            .map_err(|e| SessionError::internal(&format!("Failed to add session2 to bridge: {}", e)))?;
        
        // Emit bridge created event
        // Note: BridgeEvent enum doesn't have a Created variant, emit participant added events instead
        self.emit_bridge_event(BridgeEvent::ParticipantAdded {
            bridge_id: bridge_id.clone(),
            session_id: session1.clone(),
        }).await;
        
        self.emit_bridge_event(BridgeEvent::ParticipantAdded {
            bridge_id: bridge_id.clone(),
            session_id: session2.clone(),
        }).await;
        
        Ok(bridge_id)
    }
    
    /// Create a bridge (conference) with no initial sessions
    pub async fn create_bridge(&self) -> Result<BridgeId> {
        let bridge_id = BridgeId::new();
        let conf_id = ConferenceId::from_name(&bridge_id.0);
        
        // Create conference config for bridge
        let config = ConferenceConfig {
            name: bridge_id.0.clone(),
            max_participants: 10, // Allow more than 2 for conferences
            audio_mixing_enabled: true,
            audio_sample_rate: 8000,
            audio_channels: 1,
            rtp_port_range: Some((10000, 20000)),
            timeout: None,
        };
        
        // Create the conference
        self.conference_manager.create_named_conference(conf_id.clone(), config).await
            .map_err(|e| SessionError::internal(&format!("Failed to create bridge: {}", e)))?;
        
        // No emit for bridge creation without participants
        // The BridgeEvent enum only has participant-related events and destruction
        
        Ok(bridge_id)
    }
    
    /// Destroy a bridge
    pub async fn destroy_bridge(&self, bridge_id: &BridgeId) -> Result<()> {
        let conf_id = ConferenceId::from_name(&bridge_id.0);
        self.conference_manager.terminate_conference(&conf_id).await
            .map_err(|e| SessionError::internal(&format!("Failed to destroy bridge: {}", e)))?;
        
        self.emit_bridge_event(BridgeEvent::BridgeDestroyed {
            bridge_id: bridge_id.clone(),
        }).await;
        
        Ok(())
    }
    
    /// Add a session to an existing bridge
    pub async fn add_session_to_bridge(
        &self,
        bridge_id: &BridgeId,
        session_id: &SessionId,
    ) -> Result<()> {
        let conf_id = ConferenceId::from_name(&bridge_id.0);
        
        self.conference_manager.join_conference(&conf_id, session_id).await
            .map_err(|e| SessionError::internal(&format!("Failed to add session to bridge: {}", e)))?;
        
        self.emit_bridge_event(BridgeEvent::ParticipantAdded {
            bridge_id: bridge_id.clone(),
            session_id: session_id.clone(),
        }).await;
        
        Ok(())
    }
    
    /// Remove a session from a bridge
    pub async fn remove_session_from_bridge(
        &self,
        bridge_id: &BridgeId,
        session_id: &SessionId,
    ) -> Result<()> {
        let conf_id = ConferenceId::from_name(&bridge_id.0);
        self.conference_manager.leave_conference(&conf_id, session_id).await
            .map_err(|e| SessionError::internal(&format!("Failed to remove session from bridge: {}", e)))?;
        
        self.emit_bridge_event(BridgeEvent::ParticipantRemoved {
            bridge_id: bridge_id.clone(),
            session_id: session_id.clone(),
            reason: "Manually removed from bridge".to_string(),
        }).await;
        
        Ok(())
    }
    
    /// Get the bridge a session is part of
    pub async fn get_session_bridge(&self, session_id: &SessionId) -> Result<Option<BridgeId>> {
        // Iterate through all conferences to find which one contains this session
        let conference_ids = self.conference_manager.list_conferences().await
            .map_err(|e| SessionError::internal(&format!("Failed to list conferences: {}", e)))?;
            
        for conf_id in conference_ids {
            let participants = self.conference_manager.list_participants(&conf_id).await
                .map_err(|e| SessionError::internal(&format!("Failed to list participants: {}", e)))?;
                
            if participants.iter().any(|p| &p.session_id == session_id) {
                return Ok(Some(BridgeId(conf_id.0)));
            }
        }
        
        Ok(None)
    }
    
    /// Get information about a bridge
    pub async fn get_bridge_info(&self, bridge_id: &BridgeId) -> Result<Option<BridgeInfo>> {
        let conf_id = ConferenceId::from_name(&bridge_id.0);
        
        // Check if conference exists
        if !self.conference_manager.conference_exists(&conf_id).await {
            return Ok(None);
        }
        
        // Get participants
        let participants = self.conference_manager.list_participants(&conf_id).await
            .map_err(|e| SessionError::internal(&format!("Failed to list participants: {}", e)))?;
            
        let session_ids: Vec<SessionId> = participants.iter()
            .map(|p| p.session_id.clone())
            .collect();
            
        Ok(Some(BridgeInfo {
            id: bridge_id.clone(),
            sessions: session_ids,
            created_at: Instant::now(), // Conference doesn't track creation time yet
            participant_count: participants.len(),
        }))
    }
    
    /// List all active bridges
    pub async fn list_bridges(&self) -> Vec<BridgeInfo> {
        match self.conference_manager.list_conferences().await {
            Ok(conference_ids) => {
                let mut bridges = Vec::new();
                
                for conf_id in conference_ids {
                    // Get participants for each conference
                    if let Ok(participants) = self.conference_manager.list_participants(&conf_id).await {
                        // Only include conferences that act as bridges (2-party conferences)
                        if participants.len() <= 2 {
                            let session_ids: Vec<SessionId> = participants.iter()
                                .map(|p| p.session_id.clone())
                                .collect();
                                
                            bridges.push(BridgeInfo {
                                id: BridgeId(conf_id.0),
                                sessions: session_ids,
                                created_at: Instant::now(), // Conference doesn't track creation time yet
                                participant_count: participants.len(),
                            });
                        }
                    }
                }
                
                bridges
            }
            Err(e) => {
                tracing::error!("Failed to list conferences: {}", e);
                Vec::new()
            }
        }
    }
    
    /// Subscribe to bridge events
    pub async fn subscribe_to_bridge_events(&self) -> mpsc::UnboundedReceiver<BridgeEvent> {
        let (tx, rx) = mpsc::unbounded_channel();
        self.bridge_event_subscribers.write().await.push(tx);
        rx
    }
    
    /// Emit a bridge event to all subscribers
    pub(crate) async fn emit_bridge_event(&self, event: BridgeEvent) {
        let subscribers = self.bridge_event_subscribers.read().await;
        for subscriber in subscribers.iter() {
            // Ignore send errors (subscriber may have dropped)
            let _ = subscriber.send(event.clone());
        }
    }
} 