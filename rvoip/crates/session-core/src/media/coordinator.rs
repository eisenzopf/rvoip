//! Session Media Coordinator
//!
//! Handles automatic media lifecycle management, adapted from the proven working implementation
//! in src-old/media/coordination.rs. This coordinator automatically manages media sessions
//! based on SIP session events.

use crate::api::types::SessionId;
use super::types::*;
use super::manager::MediaManager;
use std::sync::Arc;
use tokio::sync::RwLock;
use std::collections::HashMap;

/// Coordinates media lifecycle with SIP session events
/// 
/// This will be adapted from the working SessionMediaCoordinator in 
/// src-old/media/coordination.rs to integrate with the current session event system.
pub struct SessionMediaCoordinator {
    /// Media manager for actual media operations
    media_manager: Arc<MediaManager>,
    
    /// Mapping from SIP session ID to media session ID
    session_mapping: Arc<RwLock<HashMap<SessionId, MediaSessionId>>>,
    
    /// Event handlers for media events (to be connected to session events)
    event_handlers: Vec<Arc<dyn MediaEventHandler>>,
}

impl SessionMediaCoordinator {
    /// Create a new coordinator with the specified media manager
    pub fn new(media_manager: Arc<MediaManager>) -> Self {
        Self {
            media_manager,
            session_mapping: Arc::new(RwLock::new(HashMap::new())),
            event_handlers: Vec::new(),
        }
    }
    
    /// Add an event handler for media events
    pub fn add_event_handler(&mut self, handler: Arc<dyn MediaEventHandler>) {
        self.event_handlers.push(handler);
    }
    
    /// Handle SIP session created event
    /// 
    /// This will be expanded with logic from src-old/media/coordination.rs
    /// to automatically create media sessions when SIP sessions are established.
    pub async fn on_session_created(&self, session_id: &SessionId) -> super::MediaResult<()> {
        tracing::debug!("Handling session created event for: {}", session_id);
        
        // TODO: Adapt from src-old/media/coordination.rs session creation logic
        let media_session = self.media_manager.create_media_session(session_id).await?;
        
        let mut mapping = self.session_mapping.write().await;
        mapping.insert(session_id.clone(), media_session.session_id.clone());
        
        // Notify event handlers
        let event = MediaEvent::SessionEstablished {
            session_id: media_session.session_id.clone(),
            info: media_session,
        };
        self.notify_handlers(&event).await;
        
        tracing::info!("Media session created and mapped for SIP session: {}", session_id);
        Ok(())
    }
    
    /// Handle SIP session answered event
    pub async fn on_session_answered(&self, session_id: &SessionId, answer_sdp: &str) -> super::MediaResult<()> {
        tracing::debug!("Handling session answered event for: {}", session_id);
        
        // TODO: Adapt from src-old/media/coordination.rs answer processing logic
        // TODO: Update media session with negotiated SDP
        
        tracing::warn!("Session answered handling not yet implemented - will be added in Phase 14.2");
        Ok(())
    }
    
    /// Handle SIP session terminated event
    pub async fn on_session_terminated(&self, session_id: &SessionId) -> super::MediaResult<()> {
        tracing::debug!("Handling session terminated event for: {}", session_id);
        
        // TODO: Adapt from src-old/media/coordination.rs termination logic
        let mut mapping = self.session_mapping.write().await;
        if let Some(media_session_id) = mapping.remove(session_id) {
            // Notify event handlers
            let event = MediaEvent::SessionTerminated {
                session_id: media_session_id.clone(),
            };
            self.notify_handlers(&event).await;
            
            tracing::info!("Media session terminated for SIP session: {}", session_id);
        } else {
            tracing::warn!("No media session found for SIP session: {}", session_id);
        }
        
        Ok(())
    }
    
    /// Handle SIP session hold event
    pub async fn on_session_hold(&self, session_id: &SessionId) -> super::MediaResult<()> {
        tracing::debug!("Handling session hold event for: {}", session_id);
        
        // TODO: Adapt from src-old/media/coordination.rs hold logic
        // TODO: Pause media session
        
        tracing::warn!("Session hold handling not yet implemented - will be added in Phase 14.5");
        Ok(())
    }
    
    /// Handle SIP session resume event
    pub async fn on_session_resume(&self, session_id: &SessionId) -> super::MediaResult<()> {
        tracing::debug!("Handling session resume event for: {}", session_id);
        
        // TODO: Adapt from src-old/media/coordination.rs resume logic
        // TODO: Resume media session
        
        tracing::warn!("Session resume handling not yet implemented - will be added in Phase 14.5");
        Ok(())
    }
    
    /// Get media session ID for a SIP session
    pub async fn get_media_session_id(&self, session_id: &SessionId) -> Option<MediaSessionId> {
        let mapping = self.session_mapping.read().await;
        mapping.get(session_id).cloned()
    }
    
    /// Get all active session mappings
    pub async fn get_active_mappings(&self) -> HashMap<SessionId, MediaSessionId> {
        let mapping = self.session_mapping.read().await;
        mapping.clone()
    }
    
    /// Generate SDP offer for a session
    pub async fn generate_sdp_offer(&self, session_id: &SessionId) -> super::MediaResult<String> {
        self.media_manager.generate_sdp_offer(session_id).await
    }
    
    /// Process SDP answer for a session
    pub async fn process_sdp_answer(&self, session_id: &SessionId, sdp: &str) -> super::MediaResult<()> {
        self.media_manager.process_sdp_answer(session_id, sdp).await
    }
    
    /// Internal method to notify all event handlers
    async fn notify_handlers(&self, event: &MediaEvent) {
        for handler in &self.event_handlers {
            if let Err(e) = handler.handle_media_event(event.clone()).await {
                tracing::error!("Media event handler error: {}", e);
            }
        }
    }
}

impl std::fmt::Debug for SessionMediaCoordinator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionMediaCoordinator")
            .field("event_handlers_count", &self.event_handlers.len())
            .field("session_mapping", &"<async>")
            .finish_non_exhaustive()
    }
}

/// Trait for handling media events
#[async_trait::async_trait]
pub trait MediaEventHandler: Send + Sync {
    /// Handle a media event
    async fn handle_media_event(&self, event: MediaEvent) -> Result<(), Box<dyn std::error::Error + Send + Sync>>;
}

/// Default media event handler that just logs events
#[derive(Debug)]
pub struct LoggingMediaEventHandler;

#[async_trait::async_trait]
impl MediaEventHandler for LoggingMediaEventHandler {
    async fn handle_media_event(&self, event: MediaEvent) -> Result<(), Box<dyn std::error::Error + Send + Sync>> {
        match event {
            MediaEvent::SessionEstablished { session_id, .. } => {
                tracing::info!("Media session established: {}", session_id);
            }
            MediaEvent::SessionTerminated { session_id } => {
                tracing::info!("Media session terminated: {}", session_id);
            }
            MediaEvent::QualityUpdate { session_id, metrics } => {
                tracing::debug!("Quality update for {}: {:?}", session_id, metrics);
            }
            MediaEvent::DtmfDetected { session_id, tone, duration } => {
                tracing::info!("DTMF detected in {}: {} ({}ms)", session_id, tone, duration);
            }
            MediaEvent::Error { session_id, error } => {
                tracing::error!("Media error in {}: {}", session_id, error);
            }
            
            // NEW: Handle zero-copy RTP processing events (Phase 16.2)
            MediaEvent::RtpPacketProcessed { session_id, processing_type, performance_metrics } => {
                tracing::debug!("RTP packet processed for {}: {:?} ({}% allocation reduction)", 
                               session_id, processing_type, performance_metrics.allocation_reduction_percentage);
            }
            
            MediaEvent::RtpProcessingModeChanged { session_id, old_mode, new_mode } => {
                tracing::info!("RTP processing mode changed for {}: {:?} → {:?}", 
                              session_id, old_mode, new_mode);
            }
            
            MediaEvent::RtpProcessingError { session_id, error, fallback_applied } => {
                if fallback_applied {
                    tracing::warn!("RTP processing error for {} with fallback: {}", session_id, error);
                } else {
                    tracing::error!("RTP processing error for {}: {}", session_id, error);
                }
            }
            
            MediaEvent::RtpBufferPoolUpdate { stats } => {
                tracing::debug!("RTP buffer pool update: {}% efficiency ({}/{} buffers)",
                               stats.efficiency_percentage, stats.in_use_buffers, stats.total_buffers);
            }
        }
        Ok(())
    }
}

/// Builder for SessionMediaCoordinator
pub struct SessionMediaCoordinatorBuilder {
    media_manager: Option<Arc<MediaManager>>,
    event_handlers: Vec<Arc<dyn MediaEventHandler>>,
}

impl SessionMediaCoordinatorBuilder {
    /// Create a new builder
    pub fn new() -> Self {
        Self {
            media_manager: None,
            event_handlers: Vec::new(),
        }
    }
    
    /// Set the media manager
    pub fn with_media_manager(mut self, manager: Arc<MediaManager>) -> Self {
        self.media_manager = Some(manager);
        self
    }
    
    /// Add an event handler
    pub fn with_event_handler(mut self, handler: Arc<dyn MediaEventHandler>) -> Self {
        self.event_handlers.push(handler);
        self
    }
    
    /// Add logging event handler
    pub fn with_logging(self) -> Self {
        self.with_event_handler(Arc::new(LoggingMediaEventHandler))
    }
    
    /// Build the coordinator
    pub fn build(self) -> Result<SessionMediaCoordinator, &'static str> {
        let media_manager = self.media_manager
            .ok_or("MediaManager is required")?;
        
        let mut coordinator = SessionMediaCoordinator::new(media_manager);
        
        for handler in self.event_handlers {
            coordinator.add_event_handler(handler);
        }
        
        Ok(coordinator)
    }
}

impl std::fmt::Debug for SessionMediaCoordinatorBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionMediaCoordinatorBuilder")
            .field("media_manager", &self.media_manager.is_some())
            .field("event_handlers_count", &self.event_handlers.len())
            .finish()
    }
}

impl Default for SessionMediaCoordinatorBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::media::manager::MediaManager;
    
    #[tokio::test]
    async fn test_coordinator_creation() {
        let local_addr = "127.0.0.1:8000".parse().unwrap();
        let media_manager = Arc::new(MediaManager::new(local_addr));
        let coordinator = SessionMediaCoordinator::new(media_manager);
        
        let mappings = coordinator.get_active_mappings().await;
        assert!(mappings.is_empty());
    }
    
    #[tokio::test]
    async fn test_session_lifecycle() {
        let local_addr = "127.0.0.1:8000".parse().unwrap();
        let media_manager = Arc::new(MediaManager::with_port_range(local_addr, 10000, 20000));
        let coordinator = SessionMediaCoordinator::new(media_manager);
        let session_id = SessionId::new();
        
        // Test session creation
        let result = coordinator.on_session_created(&session_id).await;
        assert!(result.is_ok());
        
        // Check mapping was created
        let media_session_id = coordinator.get_media_session_id(&session_id).await;
        assert!(media_session_id.is_some());
        
        // Test session termination
        let result = coordinator.on_session_terminated(&session_id).await;
        assert!(result.is_ok());
        
        // Check mapping was removed
        let media_session_id = coordinator.get_media_session_id(&session_id).await;
        assert!(media_session_id.is_none());
    }
    
    #[tokio::test]
    async fn test_sdp_operations() {
        let local_addr = "127.0.0.1:8000".parse().unwrap();
        let media_manager = Arc::new(MediaManager::with_port_range(local_addr, 10000, 20000));
        let coordinator = SessionMediaCoordinator::new(media_manager);
        let session_id = SessionId::new();
        
        // First create a media session
        let _result = coordinator.on_session_created(&session_id).await.unwrap();
        
        // Test SDP offer generation
        let sdp_result = coordinator.generate_sdp_offer(&session_id).await;
        assert!(sdp_result.is_ok());
        
        let sdp = sdp_result.unwrap();
        assert!(sdp.contains("m=audio"));
    }
} 