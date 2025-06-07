//! Media-Session Event Bridge
//!
//! Bridges media events with the session event system, enabling automatic
//! media lifecycle management based on SIP session events.

use crate::api::types::SessionId;
use super::types::*;
use super::coordinator::SessionMediaCoordinator;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Bridges media events with session events
/// 
/// This bridge coordinates between the session event system and media management,
/// ensuring that media sessions are automatically managed based on SIP session lifecycle.
#[derive(Debug)]
pub struct MediaBridge {
    /// Media coordinator for handling session events
    coordinator: Arc<SessionMediaCoordinator>,
    
    /// Whether the bridge is currently active
    active: Arc<RwLock<bool>>,
}

impl MediaBridge {
    /// Create a new media bridge with the specified coordinator
    pub fn new(coordinator: Arc<SessionMediaCoordinator>) -> Self {
        Self {
            coordinator,
            active: Arc::new(RwLock::new(false)),
        }
    }
    
    /// Start the bridge (enable event processing)
    pub async fn start(&self) {
        let mut active = self.active.write().await;
        *active = true;
        tracing::info!("Media bridge started");
    }
    
    /// Stop the bridge (disable event processing)
    pub async fn stop(&self) {
        let mut active = self.active.write().await;
        *active = false;
        tracing::info!("Media bridge stopped");
    }
    
    /// Check if the bridge is active
    pub async fn is_active(&self) -> bool {
        *self.active.read().await
    }
    
    /// Handle a session event and trigger appropriate media operations
    /// 
    /// This method will be integrated with the session event system to automatically
    /// manage media sessions based on SIP session lifecycle events.
    pub async fn handle_session_event(&self, event: SessionEventBridge) -> Result<(), MediaBridgeError> {
        if !self.is_active().await {
            return Ok(()); // Bridge is disabled
        }
        
        tracing::debug!("Handling session event: {:?}", event);
        
        match event {
            SessionEventBridge::SessionCreated { session_id } => {
                self.coordinator.on_session_created(&session_id)
                    .await
                    .map_err(|e| MediaBridgeError::CoordinationFailed { 
                        session_id: session_id.to_string(), 
                        reason: e.to_string() 
                    })?;
            }
            
            SessionEventBridge::SessionAnswered { session_id, answer_sdp } => {
                self.coordinator.on_session_answered(&session_id, &answer_sdp)
                    .await
                    .map_err(|e| MediaBridgeError::CoordinationFailed { 
                        session_id: session_id.to_string(), 
                        reason: e.to_string() 
                    })?;
            }
            
            SessionEventBridge::SessionTerminated { session_id } => {
                self.coordinator.on_session_terminated(&session_id)
                    .await
                    .map_err(|e| MediaBridgeError::CoordinationFailed { 
                        session_id: session_id.to_string(), 
                        reason: e.to_string() 
                    })?;
            }
            
            SessionEventBridge::SessionHold { session_id } => {
                self.coordinator.on_session_hold(&session_id)
                    .await
                    .map_err(|e| MediaBridgeError::CoordinationFailed { 
                        session_id: session_id.to_string(), 
                        reason: e.to_string() 
                    })?;
            }
            
            SessionEventBridge::SessionResume { session_id } => {
                self.coordinator.on_session_resume(&session_id)
                    .await
                    .map_err(|e| MediaBridgeError::CoordinationFailed { 
                        session_id: session_id.to_string(), 
                        reason: e.to_string() 
                    })?;
            }
        }
        
        tracing::debug!("Successfully handled session event");
        Ok(())
    }
    
    /// Generate SDP offer for a session (convenience method)
    pub async fn generate_sdp_offer(&self, session_id: &SessionId) -> Result<String, MediaBridgeError> {
        self.coordinator.generate_sdp_offer(session_id)
            .await
            .map_err(|e| MediaBridgeError::SdpGeneration { 
                session_id: session_id.to_string(), 
                reason: e.to_string() 
            })
    }
    
    /// Process SDP answer for a session (convenience method)
    pub async fn process_sdp_answer(&self, session_id: &SessionId, sdp: &str) -> Result<(), MediaBridgeError> {
        self.coordinator.process_sdp_answer(session_id, sdp)
            .await
            .map_err(|e| MediaBridgeError::SdpProcessing { 
                session_id: session_id.to_string(), 
                reason: e.to_string() 
            })
    }
    
    /// Get media session ID for a SIP session (convenience method)
    pub async fn get_media_session_id(&self, session_id: &SessionId) -> Option<MediaSessionId> {
        self.coordinator.get_media_session_id(session_id).await
    }
}

/// Session events that the media bridge can handle
/// 
/// These events will be mapped from the session-core event system to trigger
/// appropriate media operations.
#[derive(Debug, Clone)]
pub enum SessionEventBridge {
    /// SIP session was created
    SessionCreated {
        session_id: SessionId,
    },
    
    /// SIP session was answered (with SDP)
    SessionAnswered {
        session_id: SessionId,
        answer_sdp: String,
    },
    
    /// SIP session was terminated
    SessionTerminated {
        session_id: SessionId,
    },
    
    /// SIP session was put on hold
    SessionHold {
        session_id: SessionId,
    },
    
    /// SIP session was resumed from hold
    SessionResume {
        session_id: SessionId,
    },
}

/// Errors that can occur in the media bridge
#[derive(Debug, thiserror::Error)]
pub enum MediaBridgeError {
    #[error("Media coordination failed for session {session_id}: {reason}")]
    CoordinationFailed {
        session_id: String,
        reason: String,
    },
    
    #[error("SDP generation failed for session {session_id}: {reason}")]
    SdpGeneration {
        session_id: String,
        reason: String,
    },
    
    #[error("SDP processing failed for session {session_id}: {reason}")]
    SdpProcessing {
        session_id: String,
        reason: String,
    },
    
    #[error("Bridge is not active")]
    BridgeInactive,
}

/// Builder for MediaBridge
#[derive(Debug)]
pub struct MediaBridgeBuilder {
    coordinator: Option<Arc<SessionMediaCoordinator>>,
    auto_start: bool,
}

impl MediaBridgeBuilder {
    /// Create a new builder
    pub fn new() -> Self {
        Self {
            coordinator: None,
            auto_start: true,
        }
    }
    
    /// Set the media coordinator
    pub fn with_coordinator(mut self, coordinator: Arc<SessionMediaCoordinator>) -> Self {
        self.coordinator = Some(coordinator);
        self
    }
    
    /// Set whether to auto-start the bridge
    pub fn auto_start(mut self, auto_start: bool) -> Self {
        self.auto_start = auto_start;
        self
    }
    
    /// Build the media bridge
    pub async fn build(self) -> Result<MediaBridge, &'static str> {
        let coordinator = self.coordinator
            .ok_or("SessionMediaCoordinator is required")?;
        
        let bridge = MediaBridge::new(coordinator);
        
        if self.auto_start {
            bridge.start().await;
        }
        
        Ok(bridge)
    }
}

impl Default for MediaBridgeBuilder {
    fn default() -> Self {
        Self::new()
    }
}

/// Trait for components that can provide media bridge integration
/// 
/// This trait will be implemented by SessionManager to provide media bridge access
/// for automatic media lifecycle management.
#[async_trait::async_trait]
pub trait MediaBridgeProvider {
    /// Get the media bridge for this component
    async fn get_media_bridge(&self) -> Option<Arc<MediaBridge>>;
    
    /// Set up media bridge integration
    async fn setup_media_bridge(&self, bridge: Arc<MediaBridge>) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}

/// Helper function to create a media bridge from a media coordinator
pub async fn create_media_bridge(coordinator: Arc<SessionMediaCoordinator>) -> Result<Arc<MediaBridge>, MediaBridgeError> {
    let bridge = MediaBridgeBuilder::new()
        .with_coordinator(coordinator)
        .auto_start(true)
        .build()
        .await
        .map_err(|e| MediaBridgeError::CoordinationFailed { 
            session_id: "bridge-creation".to_string(), 
            reason: e.to_string() 
        })?;
    
    Ok(Arc::new(bridge))
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::{MediaManager, SessionMediaCoordinator};
    
    #[tokio::test]
    async fn test_bridge_creation() {
        let local_addr = "127.0.0.1:8000".parse().unwrap();
        let media_manager = Arc::new(MediaManager::new(local_addr));
        let coordinator = Arc::new(SessionMediaCoordinator::new(media_manager));
        let bridge = MediaBridge::new(coordinator);
        
        assert!(!bridge.is_active().await);
        
        bridge.start().await;
        assert!(bridge.is_active().await);
        
        bridge.stop().await;
        assert!(!bridge.is_active().await);
    }
    
    #[tokio::test]
    async fn test_session_event_handling() {
        let local_addr = "127.0.0.1:8000".parse().unwrap();
        let media_manager = Arc::new(MediaManager::with_port_range(local_addr, 10000, 20000));
        let coordinator = Arc::new(SessionMediaCoordinator::new(media_manager));
        let bridge = MediaBridge::new(coordinator);
        
        bridge.start().await;
        
        let session_id = SessionId::new();
        let event = SessionEventBridge::SessionCreated { 
            session_id: session_id.clone() 
        };
        
        let result = bridge.handle_session_event(event).await;
        assert!(result.is_ok());
        
        // Check that media session was created
        let media_session_id = bridge.get_media_session_id(&session_id).await;
        assert!(media_session_id.is_some());
    }
    
    #[tokio::test]
    async fn test_sdp_operations() {
        let local_addr = "127.0.0.1:8000".parse().unwrap();
        let media_manager = Arc::new(MediaManager::with_port_range(local_addr, 10000, 20000));
        let coordinator = Arc::new(SessionMediaCoordinator::new(media_manager));
        let bridge = MediaBridge::new(coordinator);
        
        bridge.start().await;
        
        let session_id = SessionId::new();
        
        // First create a media session for this SIP session
        let event = SessionEventBridge::SessionCreated { 
            session_id: session_id.clone() 
        };
        let _result = bridge.handle_session_event(event).await.unwrap();
        
        // Test SDP offer generation
        let sdp_result = bridge.generate_sdp_offer(&session_id).await;
        assert!(sdp_result.is_ok());
        
        let sdp = sdp_result.unwrap();
        assert!(sdp.contains("m=audio"));
    }
    
    #[tokio::test]
    async fn test_bridge_builder() {
        let local_addr = "127.0.0.1:8000".parse().unwrap();
        let media_manager = Arc::new(MediaManager::new(local_addr));
        let coordinator = Arc::new(SessionMediaCoordinator::new(media_manager));
        
        let bridge = MediaBridgeBuilder::new()
            .with_coordinator(coordinator)
            .auto_start(false)
            .build()
            .await
            .unwrap();
        
        assert!(!bridge.is_active().await);
    }
} 