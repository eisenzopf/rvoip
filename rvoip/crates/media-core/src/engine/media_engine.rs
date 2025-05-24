//! MediaEngine - Central orchestrator for media processing
//!
//! This is the main entry point for all media processing operations.
//! It coordinates codec management, session management, audio processing,
//! and integration with other crates.

use std::sync::Arc;
use std::collections::HashMap;
use tokio::sync::RwLock;
use tracing::{debug, info, warn, error};

use crate::error::{Result, Error};
use crate::types::{DialogId, MediaSessionId, PayloadType};
use super::config::{MediaEngineConfig, EngineCapabilities, AudioCodecCapability};
use super::lifecycle::{LifecycleManager, EngineState};

/// Parameters for creating a media session
#[derive(Debug, Clone)]
pub struct MediaSessionParams {
    /// Preferred codec payload type
    pub preferred_codec: Option<PayloadType>,
    /// Enable audio processing
    pub enable_processing: bool,
    /// Custom session configuration
    pub custom_config: Option<serde_json::Value>,
}

impl MediaSessionParams {
    /// Create audio-only session parameters
    pub fn audio_only() -> Self {
        Self {
            preferred_codec: Some(0), // PCMU by default
            enable_processing: true,
            custom_config: None,
        }
    }
    
    /// Set preferred codec
    pub fn with_preferred_codec(mut self, payload_type: PayloadType) -> Self {
        self.preferred_codec = Some(payload_type);
        self
    }
    
    /// Enable/disable audio processing
    pub fn with_processing_enabled(mut self, enabled: bool) -> Self {
        self.enable_processing = enabled;
        self
    }
}

/// Handle to a media session for external operations
#[derive(Debug, Clone)]
pub struct MediaSessionHandle {
    /// Session identifier
    pub session_id: MediaSessionId,
    /// Reference to the engine for operations
    engine: Arc<MediaEngine>,
}

impl MediaSessionHandle {
    /// Create a new session handle
    fn new(session_id: MediaSessionId, engine: Arc<MediaEngine>) -> Self {
        Self { session_id, engine }
    }
    
    /// Get the session ID
    pub fn id(&self) -> &MediaSessionId {
        &self.session_id
    }
    
    /// Placeholder for future session operations
    pub async fn get_stats(&self) -> Result<serde_json::Value> {
        // TODO: Implement actual stats collection
        Ok(serde_json::json!({
            "session_id": self.session_id.as_str(),
            "status": "active"
        }))
    }
}

/// Central MediaEngine for coordinating all media processing
#[derive(Debug)]
pub struct MediaEngine {
    /// Engine configuration
    config: MediaEngineConfig,
    
    /// Lifecycle manager
    lifecycle: Arc<LifecycleManager>,
    
    /// Active media sessions
    sessions: RwLock<HashMap<MediaSessionId, MediaSessionHandle>>,
    
    /// Engine capabilities
    capabilities: EngineCapabilities,
    
    // TODO: Add component managers when implemented
    // codec_manager: Arc<CodecManager>,
    // session_manager: Arc<SessionManager>,
    // quality_monitor: Arc<QualityMonitor>,
    // audio_processor: Arc<AudioProcessor>,
}

impl MediaEngine {
    /// Create a new MediaEngine with the given configuration
    pub async fn new(config: MediaEngineConfig) -> Result<Arc<Self>> {
        info!("Creating new MediaEngine");
        
        // Create engine capabilities based on config
        let capabilities = Self::build_capabilities(&config);
        
        let engine = Arc::new(Self {
            config,
            lifecycle: Arc::new(LifecycleManager::new()),
            sessions: RwLock::new(HashMap::new()),
            capabilities,
        });
        
        debug!("MediaEngine created successfully");
        Ok(engine)
    }
    
    /// Start the MediaEngine
    pub async fn start(self: &Arc<Self>) -> Result<()> {
        info!("Starting MediaEngine");
        self.lifecycle.start().await?;
        info!("MediaEngine started and ready");
        Ok(())
    }
    
    /// Stop the MediaEngine
    pub async fn stop(&self) -> Result<()> {
        info!("Stopping MediaEngine");
        
        // Close all active sessions first
        self.close_all_sessions().await?;
        
        // Then stop the engine
        self.lifecycle.stop().await?;
        info!("MediaEngine stopped");
        Ok(())
    }
    
    /// Get the current engine state
    pub async fn state(&self) -> EngineState {
        self.lifecycle.state().await
    }
    
    /// Check if the engine is running
    pub async fn is_running(&self) -> bool {
        self.lifecycle.is_running().await
    }
    
    /// Create a new media session for a SIP dialog
    pub async fn create_media_session(
        self: &Arc<Self>,
        dialog_id: DialogId,
        params: MediaSessionParams,
    ) -> Result<MediaSessionHandle> {
        // Check if engine is running
        if !self.is_running().await {
            return Err(Error::config("MediaEngine is not running"));
        }
        
        let session_id = MediaSessionId::from_dialog(&dialog_id);
        
        // Check if session already exists
        {
            let sessions = self.sessions.read().await;
            if sessions.contains_key(&session_id) {
                return Err(Error::session_not_found(session_id.as_str()));
            }
        }
        
        info!("Creating media session for dialog: {}", dialog_id);
        
        // TODO: Create actual MediaSession with components
        // For now, create a placeholder handle
        let handle = MediaSessionHandle::new(session_id.clone(), self.clone());
        
        // Store the session
        {
            let mut sessions = self.sessions.write().await;
            sessions.insert(session_id.clone(), handle.clone());
        }
        
        debug!("Media session created: {}", session_id);
        Ok(handle)
    }
    
    /// Destroy a media session
    pub async fn destroy_media_session(&self, dialog_id: DialogId) -> Result<()> {
        let session_id = MediaSessionId::from_dialog(&dialog_id);
        
        info!("Destroying media session for dialog: {}", dialog_id);
        
        // Remove from active sessions
        {
            let mut sessions = self.sessions.write().await;
            if sessions.remove(&session_id).is_none() {
                warn!("Attempted to destroy non-existent session: {}", session_id);
                return Err(Error::session_not_found(session_id.as_str()));
            }
        }
        
        debug!("Media session destroyed: {}", session_id);
        Ok(())
    }
    
    /// Get supported codec capabilities
    pub fn get_supported_codecs(&self) -> Vec<AudioCodecCapability> {
        self.capabilities.audio_codecs.clone()
    }
    
    /// Get complete engine capabilities
    pub fn get_media_capabilities(&self) -> &EngineCapabilities {
        &self.capabilities
    }
    
    /// Get number of active sessions
    pub async fn session_count(&self) -> usize {
        self.sessions.read().await.len()
    }
    
    /// Close all active sessions
    async fn close_all_sessions(&self) -> Result<()> {
        let session_ids: Vec<MediaSessionId> = {
            let sessions = self.sessions.read().await;
            sessions.keys().cloned().collect()
        };
        
        for session_id in session_ids {
            // TODO: Gracefully close each session
            debug!("Closing session: {}", session_id);
        }
        
        // Clear all sessions
        {
            let mut sessions = self.sessions.write().await;
            sessions.clear();
        }
        
        info!("All media sessions closed");
        Ok(())
    }
    
    /// Build engine capabilities from configuration
    fn build_capabilities(config: &MediaEngineConfig) -> EngineCapabilities {
        use crate::types::{SampleRate, payload_types};
        
        let audio_codecs = config.codecs.enabled_payload_types.iter()
            .filter_map(|&pt| {
                match pt {
                    payload_types::PCMU => Some(AudioCodecCapability {
                        payload_type: pt,
                        name: "PCMU".to_string(),
                        sample_rates: vec![SampleRate::Rate8000],
                        channels: 1,
                        clock_rate: 8000,
                    }),
                    payload_types::PCMA => Some(AudioCodecCapability {
                        payload_type: pt,
                        name: "PCMA".to_string(),
                        sample_rates: vec![SampleRate::Rate8000],
                        channels: 1,
                        clock_rate: 8000,
                    }),
                    payload_types::OPUS => Some(AudioCodecCapability {
                        payload_type: pt,
                        name: "opus".to_string(),
                        sample_rates: vec![
                            SampleRate::Rate8000,
                            SampleRate::Rate16000,
                            SampleRate::Rate48000,
                        ],
                        channels: 1, // Mono for now
                        clock_rate: 48000,
                    }),
                    _ => None,
                }
            })
            .collect();
        
        EngineCapabilities {
            audio_codecs,
            audio_processing: Default::default(),
            sample_rates: vec![
                SampleRate::Rate8000,
                SampleRate::Rate16000,
                SampleRate::Rate48000,
            ],
            max_sessions: config.performance.max_sessions,
        }
    }
} 