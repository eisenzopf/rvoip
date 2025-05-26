use std::sync::Arc;
use std::collections::HashMap;
use tokio::sync::RwLock;
use anyhow::Result;
use tracing::{info, debug, warn, error};

// Import media-core components
use rvoip_media_core::prelude::*;

use crate::session::SessionId;
use crate::dialog::DialogId;
use crate::sdp::SessionDescription;
use super::{MediaManager, MediaConfig, AudioCodecType, MediaEvent};

/// SessionMediaCoordinator manages the mapping between SIP sessions and media sessions
/// This implements the coordination layer between session-core and media-core
pub struct SessionMediaCoordinator {
    /// Reference to the media manager
    media_manager: Arc<MediaManager>,
    
    /// Session to media session mapping
    session_media_map: Arc<RwLock<HashMap<SessionId, MediaSessionId>>>,
    
    /// Media session to session mapping (reverse lookup)
    media_session_map: Arc<RwLock<HashMap<MediaSessionId, SessionId>>>,
    
    /// Active media configurations
    media_configs: Arc<RwLock<HashMap<SessionId, MediaConfig>>>,
}

impl SessionMediaCoordinator {
    /// Create a new SessionMediaCoordinator
    pub async fn new(media_manager: Arc<MediaManager>) -> Result<Self> {
        debug!("Creating SessionMediaCoordinator");
        
        Ok(Self {
            media_manager,
            session_media_map: Arc::new(RwLock::new(HashMap::new())),
            media_session_map: Arc::new(RwLock::new(HashMap::new())),
            media_configs: Arc::new(RwLock::new(HashMap::new())),
        })
    }
    
    /// Create media session for a SIP session with automatic lifecycle management
    pub async fn create_media_for_session(&self, session_id: &SessionId, config: MediaConfig) -> Result<MediaSessionId> {
        debug!("Creating media for session: {} with config: {:?}", session_id, config);
        
        // Check if media already exists for this session
        {
            let session_map = self.session_media_map.read().await;
            if let Some(existing_media_id) = session_map.get(session_id) {
                warn!("Media session already exists for session {}: {}", session_id, existing_media_id);
                return Ok(existing_media_id.clone());
            }
        }
        
        // Create media session through MediaManager
        let media_session_id = self.media_manager.create_media_session(config.clone()).await
            .map_err(|e| anyhow::anyhow!("Failed to create media session: {}", e))?;
        
        // Store mappings
        {
            let mut session_map = self.session_media_map.write().await;
            session_map.insert(session_id.clone(), media_session_id.clone());
        }
        
        {
            let mut media_map = self.media_session_map.write().await;
            media_map.insert(media_session_id.clone(), session_id.clone());
        }
        
        {
            let mut configs = self.media_configs.write().await;
            configs.insert(session_id.clone(), config);
        }
        
        info!("✅ Created media session {} for SIP session {}", media_session_id, session_id);
        Ok(media_session_id)
    }
    
    /// Start media for a session with automatic coordination
    pub async fn start_media_for_session(&self, session_id: &SessionId) -> Result<()> {
        debug!("Starting media for session: {}", session_id);
        
        let media_session_id = {
            let session_map = self.session_media_map.read().await;
            session_map.get(session_id).cloned()
                .ok_or_else(|| anyhow::anyhow!("No media session found for session {}", session_id))?
        };
        
        // Start media through MediaManager
        self.media_manager.start_media(session_id, &media_session_id).await
            .map_err(|e| anyhow::anyhow!("Failed to start media: {}", e))?;
        
        info!("✅ Started media for session {} (media session: {})", session_id, media_session_id);
        Ok(())
    }
    
    /// Pause media for a session with automatic coordination
    pub async fn pause_media_for_session(&self, session_id: &SessionId) -> Result<()> {
        debug!("Pausing media for session: {}", session_id);
        
        let media_session_id = {
            let session_map = self.session_media_map.read().await;
            session_map.get(session_id).cloned()
                .ok_or_else(|| anyhow::anyhow!("No media session found for session {}", session_id))?
        };
        
        // Pause media through MediaManager
        self.media_manager.pause_media(&media_session_id).await
            .map_err(|e| anyhow::anyhow!("Failed to pause media: {}", e))?;
        
        info!("✅ Paused media for session {} (media session: {})", session_id, media_session_id);
        Ok(())
    }
    
    /// Resume media for a session with automatic coordination
    pub async fn resume_media_for_session(&self, session_id: &SessionId) -> Result<()> {
        debug!("Resuming media for session: {}", session_id);
        
        let media_session_id = {
            let session_map = self.session_media_map.read().await;
            session_map.get(session_id).cloned()
                .ok_or_else(|| anyhow::anyhow!("No media session found for session {}", session_id))?
        };
        
        // Resume media through MediaManager
        self.media_manager.resume_media(&media_session_id).await
            .map_err(|e| anyhow::anyhow!("Failed to resume media: {}", e))?;
        
        info!("✅ Resumed media for session {} (media session: {})", session_id, media_session_id);
        Ok(())
    }
    
    /// Stop media for a session with automatic cleanup
    pub async fn stop_media_for_session(&self, session_id: &SessionId, reason: String) -> Result<()> {
        debug!("Stopping media for session: {} (reason: {})", session_id, reason);
        
        let media_session_id = {
            let mut session_map = self.session_media_map.write().await;
            session_map.remove(session_id)
        };
        
        if let Some(media_session_id) = media_session_id {
            // Remove reverse mapping
            {
                let mut media_map = self.media_session_map.write().await;
                media_map.remove(&media_session_id);
            }
            
            // Remove config
            {
                let mut configs = self.media_configs.write().await;
                configs.remove(session_id);
            }
            
            // Stop media through MediaManager
            self.media_manager.stop_media(&media_session_id, reason).await
                .map_err(|e| anyhow::anyhow!("Failed to stop media: {}", e))?;
            
            info!("✅ Stopped media for session {} (media session: {})", session_id, media_session_id);
        } else {
            warn!("No media session found for session {} during stop", session_id);
        }
        
        Ok(())
    }
    
    /// Update media direction for a session
    pub async fn update_media_direction(&self, session_id: &SessionId, direction: crate::sdp::SdpDirection) -> Result<()> {
        debug!("Updating media direction for session: {} to {:?}", session_id, direction);
        
        let media_session_id = {
            let session_map = self.session_media_map.read().await;
            session_map.get(session_id).cloned()
                .ok_or_else(|| anyhow::anyhow!("No media session found for session {}", session_id))?
        };
        
        // Update direction through MediaManager
        self.media_manager.update_media_direction(&media_session_id, direction).await
            .map_err(|e| anyhow::anyhow!("Failed to update media direction: {}", e))?;
        
        info!("✅ Updated media direction for session {} to {:?}", session_id, direction);
        Ok(())
    }
    
    /// Get quality metrics for a session
    pub async fn get_quality_metrics_for_session(&self, session_id: &SessionId) -> Result<QualityMetrics> {
        debug!("Getting quality metrics for session: {}", session_id);
        
        let media_session_id = {
            let session_map = self.session_media_map.read().await;
            session_map.get(session_id).cloned()
                .ok_or_else(|| anyhow::anyhow!("No media session found for session {}", session_id))?
        };
        
        // Get metrics through MediaManager
        let metrics = self.media_manager.get_quality_metrics(&media_session_id).await
            .map_err(|e| anyhow::anyhow!("Failed to get quality metrics: {}", e))?;
        
        debug!("Got quality metrics for session {}: {:?}", session_id, metrics);
        Ok(metrics)
    }
    
    /// Create media configuration from SDP negotiation
    pub async fn create_media_config_from_sdp(&self, sdp: &SessionDescription, preferred_codec: AudioCodecType) -> Result<MediaConfig> {
        debug!("Creating media config from SDP with preferred codec: {:?}", preferred_codec);
        
        // Create media config from SDP and codec preferences
        let config = MediaConfig::from_sdp_and_codec(sdp, preferred_codec);
        
        debug!("Created media config: {:?}", config);
        Ok(config)
    }
    
    /// Get supported codecs from media-core
    pub async fn get_supported_codecs(&self) -> Vec<PayloadType> {
        self.media_manager.get_supported_codecs().await
    }
    
    /// Get media engine capabilities
    pub async fn get_media_capabilities(&self) -> EngineCapabilities {
        self.media_manager.get_capabilities().await
    }
    
    /// Get media session ID for a session
    pub async fn get_media_session_id(&self, session_id: &SessionId) -> Option<MediaSessionId> {
        let session_map = self.session_media_map.read().await;
        session_map.get(session_id).cloned()
    }
    
    /// Get session ID for a media session
    pub async fn get_session_id(&self, media_session_id: &MediaSessionId) -> Option<SessionId> {
        let media_map = self.media_session_map.read().await;
        media_map.get(media_session_id).cloned()
    }
    
    /// Get media configuration for a session
    pub async fn get_media_config(&self, session_id: &SessionId) -> Option<MediaConfig> {
        let configs = self.media_configs.read().await;
        configs.get(session_id).cloned()
    }
    
    /// Check if session has active media
    pub async fn has_active_media(&self, session_id: &SessionId) -> bool {
        let session_map = self.session_media_map.read().await;
        session_map.contains_key(session_id)
    }
    
    /// Get all active sessions with media
    pub async fn get_active_media_sessions(&self) -> Vec<SessionId> {
        let session_map = self.session_media_map.read().await;
        session_map.keys().cloned().collect()
    }
    
    /// Setup RTP relay between two sessions
    pub async fn setup_rtp_relay(&self, session_a_id: &SessionId, session_b_id: &SessionId) -> Result<super::RelayId> {
        debug!("Setting up RTP relay between sessions: {} <-> {}", session_a_id, session_b_id);
        
        // Verify both sessions have media
        let has_media_a = self.has_active_media(session_a_id).await;
        let has_media_b = self.has_active_media(session_b_id).await;
        
        if !has_media_a {
            return Err(anyhow::anyhow!("Session {} does not have active media", session_a_id));
        }
        
        if !has_media_b {
            return Err(anyhow::anyhow!("Session {} does not have active media", session_b_id));
        }
        
        // Setup relay through MediaManager
        let relay_id = self.media_manager.setup_rtp_relay(session_a_id, session_b_id).await
            .map_err(|e| anyhow::anyhow!("Failed to setup RTP relay: {}", e))?;
        
        info!("✅ Setup RTP relay {} between sessions {} <-> {}", relay_id.0, session_a_id, session_b_id);
        Ok(relay_id)
    }
    
    /// Teardown RTP relay
    pub async fn teardown_rtp_relay(&self, relay_id: &super::RelayId) -> Result<()> {
        debug!("Tearing down RTP relay: {}", relay_id.0);
        
        // Teardown relay through MediaManager
        self.media_manager.teardown_rtp_relay(relay_id).await
            .map_err(|e| anyhow::anyhow!("Failed to teardown RTP relay: {}", e))?;
        
        info!("✅ Tore down RTP relay: {}", relay_id.0);
        Ok(())
    }
    
    /// Propagate media events to session layer
    pub async fn propagate_media_event(&self, event: MediaEvent) -> Result<()> {
        debug!("Propagating media event: {:?}", event);
        
        match event {
            MediaEvent::MediaStarted { session_id, media_session_id, config } => {
                info!("📡 Media started event: session {} -> media {}", session_id, media_session_id);
                // TODO: Emit session event for media started
            },
            MediaEvent::MediaStopped { session_id, media_session_id, reason } => {
                info!("📡 Media stopped event: session {} -> media {} ({})", session_id, media_session_id, reason);
                // TODO: Emit session event for media stopped
            },
            MediaEvent::MediaQualityChanged { session_id, media_session_id, metrics } => {
                debug!("📡 Media quality changed: session {} -> media {} ({:?})", session_id, media_session_id, metrics);
                // TODO: Emit session event for quality change
            },
            MediaEvent::MediaFailed { session_id, media_session_id, error } => {
                error!("📡 Media failed event: session {} -> media {} ({})", session_id, media_session_id, error);
                // TODO: Emit session event for media failure
            },
            MediaEvent::RelayEstablished { relay_id, session_a_id, session_b_id } => {
                info!("📡 RTP relay established: {} ({} <-> {})", relay_id.0, session_a_id, session_b_id);
                // TODO: Emit session event for relay established
            },
            MediaEvent::RelayTerminated { relay_id, reason } => {
                info!("📡 RTP relay terminated: {} ({})", relay_id.0, reason);
                // TODO: Emit session event for relay terminated
            },
        }
        
        Ok(())
    }
    
    /// Shutdown the coordinator with cleanup
    pub async fn shutdown(&self) -> Result<()> {
        info!("Shutting down SessionMediaCoordinator");
        
        // Get all active sessions
        let active_sessions = self.get_active_media_sessions().await;
        
        // Stop media for all active sessions
        for session_id in active_sessions {
            if let Err(e) = self.stop_media_for_session(&session_id, "Coordinator shutdown".to_string()).await {
                warn!("Failed to stop media for session {} during shutdown: {}", session_id, e);
            }
        }
        
        info!("✅ SessionMediaCoordinator shutdown complete");
        Ok(())
    }
} 