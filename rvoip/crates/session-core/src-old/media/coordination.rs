//! Media Coordination for Session Core
//!
//! This module provides coordination between session-core and media-core,
//! implementing the automatic media lifecycle management that was a key
//! architectural goal.

use std::sync::Arc;
use std::collections::HashMap;
use tokio::sync::RwLock;
use anyhow::Result;
use tracing::{info, debug, warn, error};

use crate::session::SessionId;
use crate::sdp::SdpSession;
use crate::media::{MediaManager, MediaConfig, MediaEvent, MediaSessionId};

// Import media-core types
use rvoip_media_core::prelude::*;

/// Session Media Coordinator
/// 
/// This component implements automatic media coordination for sessions,
/// ensuring that media setup, pause, resume, and cleanup happen automatically
/// based on session state changes.
pub struct SessionMediaCoordinator {
    /// Media manager for actual media operations
    media_manager: Arc<MediaManager>,
    
    /// Session to media session mapping
    session_media_map: Arc<RwLock<HashMap<SessionId, MediaSessionId>>>,
    
    /// Media configuration cache
    media_configs: Arc<RwLock<HashMap<SessionId, MediaConfig>>>,
}

impl SessionMediaCoordinator {
    /// Create a new session media coordinator
    pub async fn new() -> Result<Self> {
        let media_manager = Arc::new(MediaManager::new().await?);
        
        Ok(Self {
            media_manager,
            session_media_map: Arc::new(RwLock::new(HashMap::new())),
            media_configs: Arc::new(RwLock::new(HashMap::new())),
        })
    }
    
    /// Create a new coordinator with existing media manager
    pub fn with_media_manager(media_manager: Arc<MediaManager>) -> Self {
        Self {
            media_manager,
            session_media_map: Arc::new(RwLock::new(HashMap::new())),
            media_configs: Arc::new(RwLock::new(HashMap::new())),
        }
    }
    
    /// Automatically set up media for a session
    pub async fn setup_media_for_session(
        &self,
        session_id: &SessionId,
        media_config: MediaConfig,
    ) -> Result<MediaSessionId> {
        info!("🎵 Setting up media for session: {}", session_id);
        
        // Create media session
        let media_session_id = self.media_manager.create_media_session(media_config.clone()).await?;
        
        // Start media
        self.media_manager.start_media(session_id, &media_session_id).await?;
        
        // Store mappings
        {
            let mut session_map = self.session_media_map.write().await;
            session_map.insert(session_id.clone(), media_session_id.clone());
        }
        
        {
            let mut config_map = self.media_configs.write().await;
            config_map.insert(session_id.clone(), media_config);
        }
        
        info!("✅ Media automatically set up for session: {} -> {}", session_id, media_session_id);
        Ok(media_session_id)
    }
    
    /// Automatically pause media for a session
    pub async fn pause_media_for_session(&self, session_id: &SessionId) -> Result<()> {
        info!("🎵 Pausing media for session: {}", session_id);
        
        let media_session_id = {
            let session_map = self.session_media_map.read().await;
            session_map.get(session_id).cloned()
        };
        
        if let Some(media_session_id) = media_session_id {
            self.media_manager.pause_media(&media_session_id).await?;
            info!("✅ Media automatically paused for session: {}", session_id);
        } else {
            warn!("No media session found for session: {}", session_id);
        }
        
        Ok(())
    }
    
    /// Automatically resume media for a session
    pub async fn resume_media_for_session(&self, session_id: &SessionId) -> Result<()> {
        info!("🎵 Resuming media for session: {}", session_id);
        
        let media_session_id = {
            let session_map = self.session_media_map.read().await;
            session_map.get(session_id).cloned()
        };
        
        if let Some(media_session_id) = media_session_id {
            self.media_manager.resume_media(&media_session_id).await?;
            info!("✅ Media automatically resumed for session: {}", session_id);
        } else {
            warn!("No media session found for session: {}", session_id);
        }
        
        Ok(())
    }
    
    /// Automatically clean up media for a session
    pub async fn cleanup_media_for_session(&self, session_id: &SessionId) -> Result<()> {
        info!("🎵 Cleaning up media for session: {}", session_id);
        
        let media_session_id = {
            let mut session_map = self.session_media_map.write().await;
            session_map.remove(session_id)
        };
        
        if let Some(media_session_id) = media_session_id {
            self.media_manager.stop_media(&media_session_id, "Session ended".to_string()).await?;
            
            // Remove config
            {
                let mut config_map = self.media_configs.write().await;
                config_map.remove(session_id);
            }
            
            info!("✅ Media automatically cleaned up for session: {}", session_id);
        } else {
            warn!("No media session found for session: {}", session_id);
        }
        
        Ok(())
    }
    
    /// Get media session ID for a session
    pub async fn get_media_session_id(&self, session_id: &SessionId) -> Option<MediaSessionId> {
        let session_map = self.session_media_map.read().await;
        session_map.get(session_id).cloned()
    }
    
    /// Get media configuration for a session
    pub async fn get_media_config(&self, session_id: &SessionId) -> Option<MediaConfig> {
        let config_map = self.media_configs.read().await;
        config_map.get(session_id).cloned()
    }
    
    /// Update media configuration for a session
    pub async fn update_media_config(
        &self,
        session_id: &SessionId,
        new_config: MediaConfig,
    ) -> Result<()> {
        info!("🎵 Updating media config for session: {}", session_id);
        
        // Store new configuration
        {
            let mut config_map = self.media_configs.write().await;
            config_map.insert(session_id.clone(), new_config);
        }
        
        info!("✅ Media config updated for session: {}", session_id);
        Ok(())
    }
    
    /// Get all active media sessions
    pub async fn get_active_media_sessions(&self) -> Vec<(SessionId, MediaSessionId)> {
        let session_map = self.session_media_map.read().await;
        session_map.iter().map(|(k, v)| (k.clone(), v.clone())).collect()
    }
    
    /// Handle media events from media-core
    pub async fn handle_media_event(&self, event: MediaEvent) -> Result<()> {
        match event {
            MediaEvent::MediaStarted { session_id, media_session_id, config } => {
                info!("📞 Media started for session: {} -> {}", session_id, media_session_id);
                
                // Update our mappings - media_session_id is already a String, convert to MediaSessionId
                {
                    let mut session_map = self.session_media_map.write().await;
                    // Create a MediaSessionId from the string - this depends on media-core's implementation
                    let media_id = rvoip_media_core::MediaSessionId::new(media_session_id.clone());
                    session_map.insert(session_id.clone(), media_id);
                }
                
                {
                    let mut config_map = self.media_configs.write().await;
                    config_map.insert(session_id, config);
                }
            },
            
            MediaEvent::MediaStopped { session_id, media_session_id, reason } => {
                info!("📞 Media stopped for session: {} -> {} (reason: {})", session_id, media_session_id, reason);
                
                // Clean up mappings
                {
                    let mut session_map = self.session_media_map.write().await;
                    session_map.remove(&session_id);
                }
                
                {
                    let mut config_map = self.media_configs.write().await;
                    config_map.remove(&session_id);
                }
            },
            
            MediaEvent::MediaQualityChanged { session_id, media_session_id, metrics_summary } => {
                debug!("📊 Media quality changed for session: {} -> {} (metrics: {})", 
                       session_id, media_session_id, metrics_summary);
            },
            
            MediaEvent::MediaFailed { session_id, media_session_id, error } => {
                error!("❌ Media failed for session: {} -> {} (error: {})", 
                       session_id, media_session_id, error);
                
                // Clean up failed session
                {
                    let mut session_map = self.session_media_map.write().await;
                    session_map.remove(&session_id);
                }
            },
            
            MediaEvent::RelayEstablished { relay_id, session_a_id, session_b_id } => {
                info!("🔗 RTP relay established: {} ({} <-> {})", relay_id.0, session_a_id, session_b_id);
            },
            
            MediaEvent::RelayTerminated { relay_id, reason } => {
                info!("🔗 RTP relay terminated: {} (reason: {})", relay_id.0, reason);
            },
        }
        
        Ok(())
    }
    
    /// Shutdown the coordinator
    pub async fn shutdown(&self) -> Result<()> {
        info!("Shutting down SessionMediaCoordinator");
        
        // Clean up all active media sessions
        let active_sessions = self.get_active_media_sessions().await;
        for (session_id, _) in active_sessions {
            if let Err(e) = self.cleanup_media_for_session(&session_id).await {
                warn!("Failed to cleanup media for session {}: {}", session_id, e);
            }
        }
        
        // Shutdown media manager
        self.media_manager.shutdown().await?;
        
        info!("✅ SessionMediaCoordinator shutdown complete");
        Ok(())
    }
} 