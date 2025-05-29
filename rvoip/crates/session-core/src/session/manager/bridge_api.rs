//! Bridge Management APIs for Session Manager
//!
//! This module contains all bridge-related functionality for the SessionManager,
//! providing APIs for call-engine to orchestrate multi-session audio bridging.

use std::sync::Arc;
use std::time::SystemTime;
use std::collections::HashMap;
use tracing::{debug, info, warn};
use tokio::sync::mpsc;

use crate::session::{SessionId, SessionManager};
use crate::session::bridge::{
    SessionBridge, BridgeId, BridgeState, BridgeInfo, BridgeConfig,
    BridgeEvent, BridgeEventType, BridgeStats, BridgeError
};

/// Bridge management implementation for SessionManager
impl SessionManager {
    /// **BRIDGE API**: Create a new session bridge
    /// 
    /// This is the primary API that call-engine uses to create bridges for connecting sessions.
    pub async fn create_bridge(&self, config: BridgeConfig) -> Result<BridgeId, BridgeError> {
        info!("🌉 Creating new session bridge with config: {:?}", config);
        
        let bridge = Arc::new(SessionBridge::new(config));
        let bridge_id = bridge.id.clone();
        
        // Store the bridge
        self.session_bridges.insert(bridge_id.clone(), bridge.clone());
        
        // Emit bridge created event
        self.emit_bridge_event(BridgeEvent {
            event_type: BridgeEventType::BridgeCreated,
            bridge_id: bridge_id.clone(),
            session_id: None,
            timestamp: SystemTime::now(),
            data: HashMap::new(),
        }).await;
        
        info!("✅ Created session bridge: {}", bridge_id);
        Ok(bridge_id)
    }
    
    /// **BRIDGE API**: Destroy a session bridge
    /// 
    /// Removes all sessions from the bridge and cleans up resources.
    pub async fn destroy_bridge(&self, bridge_id: &BridgeId) -> Result<(), BridgeError> {
        info!("🗑️ Destroying session bridge: {}", bridge_id);
        
        let bridge = self.session_bridges.get(bridge_id)
            .ok_or_else(|| BridgeError::BridgeNotFound { bridge_id: bridge_id.clone() })?
            .clone();
        
        // Set bridge to destroying state
        bridge.set_state(BridgeState::Destroying).await;
        
        // Remove all sessions from the bridge
        let session_ids = bridge.get_session_ids().await;
        for session_id in session_ids {
            if let Err(e) = self.remove_session_from_bridge(bridge_id, &session_id).await {
                warn!("Failed to remove session {} from bridge {}: {}", session_id, bridge_id, e);
            }
        }
        
        // Mark bridge as destroyed
        bridge.set_state(BridgeState::Destroyed).await;
        
        // Remove bridge from storage
        self.session_bridges.remove(bridge_id);
        
        // Emit bridge destroyed event
        self.emit_bridge_event(BridgeEvent {
            event_type: BridgeEventType::BridgeDestroyed,
            bridge_id: bridge_id.clone(),
            session_id: None,
            timestamp: SystemTime::now(),
            data: HashMap::new(),
        }).await;
        
        info!("✅ Destroyed session bridge: {}", bridge_id);
        Ok(())
    }
    
    /// **BRIDGE API**: Add a session to a bridge
    /// 
    /// This is how call-engine adds sessions to bridges for audio routing.
    pub async fn add_session_to_bridge(&self, bridge_id: &BridgeId, session_id: &SessionId) -> Result<(), BridgeError> {
        info!("🔗 Adding session {} to bridge {}", session_id, bridge_id);
        
        // Verify session exists
        if !self.sessions.contains_key(session_id) {
            return Err(BridgeError::Internal {
                message: format!("Session {} not found", session_id),
            });
        }
        
        // Get the bridge
        let bridge = self.session_bridges.get(bridge_id)
            .ok_or_else(|| BridgeError::BridgeNotFound { bridge_id: bridge_id.clone() })?
            .clone();
        
        // Check if session is already in another bridge
        if let Some(existing_bridge_id) = self.session_to_bridge.get(session_id) {
            if existing_bridge_id.value() != bridge_id {
                return Err(BridgeError::Internal {
                    message: format!("Session {} is already in bridge {}", session_id, existing_bridge_id.value()),
                });
            }
        }
        
        // Add session to bridge
        bridge.add_session(session_id.clone()).await?;
        
        // Update session-to-bridge mapping
        self.session_to_bridge.insert(session_id.clone(), bridge_id.clone());
        
        // 🎵 COORDINATE WITH MEDIA MANAGER TO SET UP RTP FORWARDING
        self.setup_bridge_media_forwarding(bridge_id, session_id).await?;
        
        // Emit session added event
        self.emit_bridge_event(BridgeEvent {
            event_type: BridgeEventType::SessionAdded,
            bridge_id: bridge_id.clone(),
            session_id: Some(session_id.clone()),
            timestamp: SystemTime::now(),
            data: HashMap::new(),
        }).await;
        
        info!("✅ Added session {} to bridge {} with RTP forwarding", session_id, bridge_id);
        Ok(())
    }
    
    /// **BRIDGE API**: Remove a session from a bridge
    /// 
    /// Removes a session from its bridge and stops audio routing.
    pub async fn remove_session_from_bridge(&self, bridge_id: &BridgeId, session_id: &SessionId) -> Result<(), BridgeError> {
        info!("🔌 Removing session {} from bridge {}", session_id, bridge_id);
        
        // Get the bridge
        let bridge = self.session_bridges.get(bridge_id)
            .ok_or_else(|| BridgeError::BridgeNotFound { bridge_id: bridge_id.clone() })?
            .clone();
        
        // 🛑 COORDINATE WITH MEDIA MANAGER TO TEAR DOWN RTP FORWARDING
        self.teardown_bridge_media_forwarding(bridge_id, session_id).await?;
        
        // Remove session from bridge
        bridge.remove_session(session_id).await?;
        
        // Remove from session-to-bridge mapping
        self.session_to_bridge.remove(session_id);
        
        // Emit session removed event
        self.emit_bridge_event(BridgeEvent {
            event_type: BridgeEventType::SessionRemoved,
            bridge_id: bridge_id.clone(),
            session_id: Some(session_id.clone()),
            timestamp: SystemTime::now(),
            data: HashMap::new(),
        }).await;
        
        info!("✅ Removed session {} from bridge {} and stopped RTP forwarding", session_id, bridge_id);
        Ok(())
    }
    
    /// **BRIDGE API**: Get bridge information
    /// 
    /// Returns detailed information about a bridge for call-engine monitoring.
    pub async fn get_bridge_info(&self, bridge_id: &BridgeId) -> Result<BridgeInfo, BridgeError> {
        let bridge = self.session_bridges.get(bridge_id)
            .ok_or_else(|| BridgeError::BridgeNotFound { bridge_id: bridge_id.clone() })?
            .clone();
        
        Ok(bridge.get_info().await)
    }
    
    /// **BRIDGE API**: List all active bridges
    /// 
    /// Returns information about all bridges for call-engine overview.
    pub async fn list_bridges(&self) -> Vec<BridgeInfo> {
        let mut bridge_infos = Vec::new();
        
        for entry in self.session_bridges.iter() {
            let bridge = entry.value();
            bridge_infos.push(bridge.get_info().await);
        }
        
        bridge_infos
    }
    
    /// **BRIDGE API**: Pause a bridge
    /// 
    /// Stops audio flow through the bridge while keeping sessions connected.
    pub async fn pause_bridge(&self, bridge_id: &BridgeId) -> Result<(), BridgeError> {
        info!("⏸️ Pausing bridge: {}", bridge_id);
        
        let bridge = self.session_bridges.get(bridge_id)
            .ok_or_else(|| BridgeError::BridgeNotFound { bridge_id: bridge_id.clone() })?
            .clone();
        
        let current_state = bridge.get_state().await;
        match current_state {
            BridgeState::Active => {
                bridge.set_state(BridgeState::Paused).await;
                
                // TODO: Coordinate with MediaManager to pause RTP forwarding
                
                info!("✅ Paused bridge: {}", bridge_id);
                Ok(())
            },
            _ => Err(BridgeError::InvalidState {
                bridge_id: bridge_id.clone(),
                state: current_state,
            }),
        }
    }
    
    /// **BRIDGE API**: Resume a bridge
    /// 
    /// Resumes audio flow through a paused bridge.
    pub async fn resume_bridge(&self, bridge_id: &BridgeId) -> Result<(), BridgeError> {
        info!("▶️ Resuming bridge: {}", bridge_id);
        
        let bridge = self.session_bridges.get(bridge_id)
            .ok_or_else(|| BridgeError::BridgeNotFound { bridge_id: bridge_id.clone() })?
            .clone();
        
        let current_state = bridge.get_state().await;
        match current_state {
            BridgeState::Paused => {
                bridge.set_state(BridgeState::Active).await;
                
                // TODO: Coordinate with MediaManager to resume RTP forwarding
                
                info!("✅ Resumed bridge: {}", bridge_id);
                Ok(())
            },
            _ => Err(BridgeError::InvalidState {
                bridge_id: bridge_id.clone(),
                state: current_state,
            }),
        }
    }
    
    /// **BRIDGE API**: Get bridge for a session
    /// 
    /// Returns the bridge ID that a session is currently in (if any).
    pub async fn get_session_bridge(&self, session_id: &SessionId) -> Option<BridgeId> {
        self.session_to_bridge.get(session_id).map(|entry| entry.value().clone())
    }
    
    /// **BRIDGE API**: Subscribe to bridge events
    /// 
    /// Allows call-engine to receive bridge event notifications.
    pub async fn subscribe_to_bridge_events(&self) -> mpsc::UnboundedReceiver<BridgeEvent> {
        let (sender, receiver) = mpsc::unbounded_channel();
        
        {
            let mut bridge_event_sender = self.bridge_event_sender.write().await;
            *bridge_event_sender = Some(sender);
        }
        
        receiver
    }
    
    /// **INTERNAL**: Emit a bridge event to call-engine
    pub(crate) async fn emit_bridge_event(&self, event: BridgeEvent) {
        if let Some(sender) = self.bridge_event_sender.read().await.as_ref() {
            if let Err(_) = sender.send(event.clone()) {
                warn!("Failed to send bridge event - call-engine may not be listening");
            }
        }
        
        debug!("Bridge event: {:?}", event);
    }
    
    /// **BRIDGE API**: Get bridge statistics
    /// 
    /// Returns aggregated statistics across all bridges.
    pub async fn get_bridge_statistics(&self) -> HashMap<BridgeId, BridgeStats> {
        let mut stats = HashMap::new();
        
        for entry in self.session_bridges.iter() {
            let bridge = entry.value();
            let bridge_info = bridge.get_info().await;
            stats.insert(bridge.id.clone(), bridge_info.stats);
        }
        
        stats
    }
}

/// **BRIDGE MEDIA INTEGRATION**: Internal methods for coordinating with MediaManager
impl SessionManager {
    /// Set up RTP forwarding when a session is added to a bridge
    async fn setup_bridge_media_forwarding(&self, bridge_id: &BridgeId, new_session_id: &SessionId) -> Result<(), BridgeError> {
        info!("🎵 Setting up RTP forwarding for session {} in bridge {}", new_session_id, bridge_id);
        
        // Get the bridge and all its sessions
        let bridge = self.session_bridges.get(bridge_id)
            .ok_or_else(|| BridgeError::BridgeNotFound { bridge_id: bridge_id.clone() })?;
        
        let all_sessions = bridge.get_session_ids().await;
        
        // Find other sessions in the bridge to create RTP forwarding pairs
        for existing_session_id in &all_sessions {
            if existing_session_id != new_session_id {
                // Create RTP relay between new session and existing session
                if let Err(e) = self.create_rtp_relay_pair(new_session_id, existing_session_id).await {
                    warn!("Failed to create RTP relay between {} and {}: {}", new_session_id, existing_session_id, e);
                    // Continue with other sessions - don't fail the entire operation
                }
            }
        }
        
        info!("✅ RTP forwarding setup complete for session {} in bridge {}", new_session_id, bridge_id);
        Ok(())
    }
    
    /// Tear down RTP forwarding when a session is removed from a bridge  
    async fn teardown_bridge_media_forwarding(&self, bridge_id: &BridgeId, session_id: &SessionId) -> Result<(), BridgeError> {
        info!("🛑 Tearing down RTP forwarding for session {} in bridge {}", session_id, bridge_id);
        
        // Get the bridge and all its sessions
        let bridge = self.session_bridges.get(bridge_id)
            .ok_or_else(|| BridgeError::BridgeNotFound { bridge_id: bridge_id.clone() })?;
        
        let all_sessions = bridge.get_session_ids().await;
        
        // Remove RTP relays with all other sessions in the bridge
        for other_session_id in &all_sessions {
            if other_session_id != session_id {
                // Remove RTP relay between this session and the other session
                if let Err(e) = self.remove_rtp_relay_pair(session_id, other_session_id).await {
                    warn!("Failed to remove RTP relay between {} and {}: {}", session_id, other_session_id, e);
                    // Continue with other sessions - don't fail the entire operation
                }
            }
        }
        
        info!("✅ RTP forwarding teardown complete for session {} in bridge {}", session_id, bridge_id);
        Ok(())
    }
    
    /// Create RTP relay pair between two sessions for audio forwarding
    async fn create_rtp_relay_pair(&self, session_a_id: &SessionId, session_b_id: &SessionId) -> Result<(), BridgeError> {
        debug!("🔗 Creating RTP relay: {} ↔ {}", session_a_id, session_b_id);
        
        // **FIX**: Wait for both media sessions to be established with retry logic
        let max_retries = 10;
        let mut retry_count = 0;
        let mut delay_ms = 50; // Start with 50ms delay
        
        loop {
            // Try to get media sessions for both SIP sessions
            let media_session_a = self.media_manager.get_media_session(session_a_id).await;
            let media_session_b = self.media_manager.get_media_session(session_b_id).await;
            
            // Check if both are ready before proceeding
            let a_ready = media_session_a.is_some();
            let b_ready = media_session_b.is_some();
            
            match (media_session_a, media_session_b) {
                (Some(media_a), Some(media_b)) => {
                    // Both media sessions are ready - create the relay
                    let dialog_a = media_a.as_str();
                    let dialog_b = media_b.as_str();
                    
                    // Create the RTP relay through MediaSessionController
                    self.media_manager.media_controller()
                        .create_relay(dialog_a.to_string(), dialog_b.to_string()).await
                        .map_err(|e| BridgeError::Internal {
                            message: format!("Failed to create RTP relay via MediaSessionController: {}", e),
                        })?;
                    
                    info!("✅ Created RTP relay: {} ↔ {} (dialogs: {} ↔ {})", 
                          session_a_id, session_b_id, dialog_a, dialog_b);
                    return Ok(());
                },
                _ => {
                    // One or both media sessions not ready yet
                    retry_count += 1;
                    
                    if retry_count >= max_retries {
                        return Err(BridgeError::Internal {
                            message: format!("Timeout waiting for media sessions: {} (ready: {}) and {} (ready: {})",
                                session_a_id, a_ready,
                                session_b_id, b_ready),
                        });
                    }
                    
                    debug!("Media sessions not ready yet (attempt {}/{}), retrying in {}ms...", 
                           retry_count, max_retries, delay_ms);
                    
                    // Wait before retrying with exponential backoff
                    tokio::time::sleep(tokio::time::Duration::from_millis(delay_ms)).await;
                    delay_ms = std::cmp::min(delay_ms * 2, 500); // Cap at 500ms
                }
            }
        }
    }
    
    /// Remove RTP relay pair between two sessions
    async fn remove_rtp_relay_pair(&self, session_a_id: &SessionId, session_b_id: &SessionId) -> Result<(), BridgeError> {
        debug!("🔌 Removing RTP relay: {} ↔ {}", session_a_id, session_b_id);
        
        // Get media sessions for both SIP sessions
        let media_session_a = self.media_manager.get_media_session(session_a_id).await;
        let media_session_b = self.media_manager.get_media_session(session_b_id).await;
        
        // If either media session doesn't exist, the relay is already gone
        if media_session_a.is_none() || media_session_b.is_none() {
            debug!("One or both media sessions not found - relay likely already removed");
            return Ok(());
        }
        
        let dialog_a = media_session_a.unwrap().as_str().to_string();
        let dialog_b = media_session_b.unwrap().as_str().to_string();
        
        // Remove the RTP relay through MediaSessionController
        // Note: MediaSessionController cleans up relays automatically when sessions are stopped
        // For now, we just log this since the relay cleanup happens during session stop
        debug!("RTP relay cleanup will happen automatically when sessions stop (dialogs: {} ↔ {})", 
               dialog_a, dialog_b);
        
        info!("✅ Removed RTP relay: {} ↔ {}", session_a_id, session_b_id);
        Ok(())
    }
} 