//! Media Manager for Session-Core
//!
//! Main interface for media operations, adapted from the proven working implementation
//! in src-old/media/mod.rs. This manager coordinates between SIP sessions and media-core.

use crate::api::types::SessionId;
use crate::errors::Result;
use super::types::*;
use super::MediaError;
use std::sync::Arc;
use tokio::sync::RwLock;

/// Main media manager for session-core
/// 
/// This will be adapted from the working MediaManager in src-old/media/mod.rs (line 280-517)
/// to integrate with the current session-core architecture.
pub struct MediaManager {
    /// Media engine implementation (real media-core or mock for testing)
    engine: Arc<dyn MediaEngine>,
    
    /// Active media sessions
    sessions: MediaSessionStorage,
    
    /// Media configuration
    config: MediaConfig,
}

impl MediaManager {
    /// Create a new MediaManager with the specified engine
    pub fn new(engine: Arc<dyn MediaEngine>, config: MediaConfig) -> Self {
        Self {
            engine,
            sessions: Arc::new(RwLock::new(std::collections::HashMap::new())),
            config,
        }
    }
    
    /// Create a MediaManager with mock engine for testing
    pub fn with_mock_engine() -> Self {
        Self::new(
            Arc::new(MockMediaEngine::new()),
            MediaConfig::default()
        )
    }
    
    /// Get supported media capabilities
    pub fn get_capabilities(&self) -> MediaCapabilities {
        self.engine.get_capabilities()
    }
    
    /// Create a new media session for a SIP session
    /// 
    /// This method will be expanded with the logic from src-old/media/mod.rs
    /// to handle real media-core integration.
    pub async fn create_media_session(&self, session_id: &SessionId) -> super::MediaResult<MediaSessionInfo> {
        tracing::debug!("Creating media session for SIP session: {}", session_id);
        
        // TODO: Adapt from src-old/media/mod.rs MediaManager implementation
        let media_session = self.engine.create_session(&self.config).await
            .map_err(|e| MediaError::MediaEngine { source: e })?;
        
        let mut sessions = self.sessions.write().await;
        sessions.insert(media_session.session_id.clone(), media_session.clone());
        
        tracing::info!("Created media session: {} for SIP session: {}", 
                      media_session.session_id, session_id);
        
        Ok(media_session)
    }
    
    /// Update a media session with new SDP (for re-INVITE, etc.)
    pub async fn update_media_session(&self, session_id: &SessionId, sdp: &str) -> super::MediaResult<()> {
        tracing::debug!("Updating media session for SIP session: {}", session_id);
        
        // TODO: Find media session by SIP session mapping
        // TODO: Adapt from src-old/media/mod.rs update logic
        
        // For now, just log - will be implemented in Phase 14.2
        tracing::warn!("Media session update not yet implemented - will be added in Phase 14.2");
        Ok(())
    }
    
    /// Terminate a media session
    pub async fn terminate_media_session(&self, session_id: &SessionId) -> super::MediaResult<()> {
        tracing::debug!("Terminating media session for SIP session: {}", session_id);
        
        // TODO: Find media session by SIP session mapping
        // TODO: Adapt from src-old/media/mod.rs termination logic
        
        // For now, just log - will be implemented in Phase 14.2
        tracing::warn!("Media session termination not yet implemented - will be added in Phase 14.2");
        Ok(())
    }
    
    /// Get media information for a session
    pub async fn get_media_info(&self, session_id: &SessionId) -> super::MediaResult<Option<MediaSessionInfo>> {
        tracing::debug!("Getting media info for SIP session: {}", session_id);
        
        // TODO: Find media session by SIP session mapping
        // TODO: Adapt from src-old/media/mod.rs info retrieval logic
        
        // For now, return empty - will be implemented in Phase 14.2
        Ok(None)
    }
    
    /// Generate SDP offer for a session
    /// 
    /// This will use the MediaConfigConverter (to be adapted from src-old/media/config.rs)
    /// to convert media capabilities into SDP format.
    pub async fn generate_sdp_offer(&self, session_id: &SessionId) -> super::MediaResult<String> {
        tracing::debug!("Generating SDP offer for session: {}", session_id);
        
        // TODO: Implement with MediaConfigConverter from src-old/media/config.rs
        // For now, return basic SDP
        let capabilities = self.engine.get_capabilities();
        let mut sdp = String::new();
        
        sdp.push_str("v=0\r\n");
        sdp.push_str("o=rvoip 0 0 IN IP4 127.0.0.1\r\n");
        sdp.push_str("s=Session\r\n");
        sdp.push_str("c=IN IP4 127.0.0.1\r\n");
        sdp.push_str("t=0 0\r\n");
        sdp.push_str(&format!("m=audio {} RTP/AVP", capabilities.port_range.0));
        
        for codec in &capabilities.codecs {
            sdp.push_str(&format!(" {}", codec.payload_type));
        }
        sdp.push_str("\r\n");
        
        for codec in &capabilities.codecs {
            sdp.push_str(&format!("a=rtpmap:{} {}/{}\r\n", 
                                codec.payload_type, codec.name, codec.sample_rate));
        }
        
        Ok(sdp)
    }
    
    /// Process SDP answer and configure media session
    pub async fn process_sdp_answer(&self, session_id: &SessionId, sdp: &str) -> super::MediaResult<()> {
        tracing::debug!("Processing SDP answer for session: {}", session_id);
        
        // TODO: Implement with MediaConfigConverter from src-old/media/config.rs
        // TODO: Update media session with negotiated parameters
        
        tracing::warn!("SDP answer processing not yet implemented - will be added in Phase 14.3");
        Ok(())
    }
    
    /// List all active media sessions
    pub async fn list_active_sessions(&self) -> Vec<MediaSessionInfo> {
        let sessions = self.sessions.read().await;
        sessions.values().cloned().collect()
    }
    
    /// Get current configuration
    pub fn get_config(&self) -> &MediaConfig {
        &self.config
    }
}

impl std::fmt::Debug for MediaManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaManager")
            .field("config", &self.config)
            .field("sessions_count", &"<async>")
            .finish_non_exhaustive()
    }
}

/// Builder for MediaManager configuration
pub struct MediaManagerBuilder {
    engine: Option<Arc<dyn MediaEngine>>,
    config: Option<MediaConfig>,
}

impl MediaManagerBuilder {
    /// Create a new builder
    pub fn new() -> Self {
        Self::default()
    }
    
    /// Set the media engine
    pub fn with_engine(mut self, engine: Arc<dyn MediaEngine>) -> Self {
        self.engine = Some(engine);
        self
    }
    
    /// Set the media configuration
    pub fn with_config(mut self, config: MediaConfig) -> Self {
        self.config = Some(config);
        self
    }
    
    /// Use mock engine for testing
    pub fn with_mock_engine(mut self) -> Self {
        self.engine = Some(Arc::new(MockMediaEngine::new()));
        self
    }
    
    /// Build the MediaManager
    pub fn build(self) -> MediaManager {
        let engine = self.engine.unwrap_or_else(|| Arc::new(MockMediaEngine::new()));
        let config = self.config.unwrap_or_default();
        
        MediaManager::new(engine, config)
    }
}

impl std::fmt::Debug for MediaManagerBuilder {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaManagerBuilder")
            .field("engine", &self.engine.is_some())
            .field("config", &self.config)
            .finish()
    }
}

impl Default for MediaManagerBuilder {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_media_manager_creation() {
        let manager = MediaManager::with_mock_engine();
        let capabilities = manager.get_capabilities();
        
        assert!(capabilities.codecs.len() >= 2);
        assert_eq!(capabilities.port_range, (10000, 20000));
    }
    
    #[tokio::test]
    async fn test_media_session_creation() {
        let manager = MediaManager::with_mock_engine();
        let session_id = SessionId::new();
        
        let result = manager.create_media_session(&session_id).await;
        assert!(result.is_ok());
        
        let media_session = result.unwrap();
        assert!(!media_session.session_id.is_empty());
        assert_eq!(media_session.local_rtp_port, Some(10000));
    }
    
    #[tokio::test]
    async fn test_sdp_generation() {
        let manager = MediaManager::with_mock_engine();
        let session_id = SessionId::new();
        
        let sdp = manager.generate_sdp_offer(&session_id).await;
        assert!(sdp.is_ok());
        
        let sdp_content = sdp.unwrap();
        assert!(sdp_content.contains("m=audio"));
        assert!(sdp_content.contains("a=rtpmap"));
    }
} 