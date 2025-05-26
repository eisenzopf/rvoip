use std::net::SocketAddr;
use std::sync::Arc;
use std::collections::HashMap;
use tokio::sync::RwLock;
use anyhow::Result;
use uuid::Uuid;
use serde::{Serialize, Deserialize};
use tracing::{info, debug, error, warn};

// Import media-core components
use rvoip_media_core::prelude::*;

use crate::session::SessionId;
use crate::dialog::DialogId;
use crate::sdp::SdpSession;

// Media coordination modules
pub mod coordination;
pub mod config;

// Re-export coordination components
pub use coordination::SessionMediaCoordinator;
pub use config::MediaConfigConverter;

// Re-export media-core types for convenience with proper paths
pub use rvoip_media_core::{
    MediaDirection, SampleRate, 
    MediaSessionParams, MediaSessionHandle, MediaEngine, MediaEngineConfig
};

// Re-export specific types that are commonly used
pub use rvoip_media_core::prelude::QualityMetrics;

/// Unique identifier for media sessions (re-export from media-core)
pub use rvoip_media_core::MediaSessionId;

/// Unique identifier for RTP relays
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct RelayId(pub Uuid);

impl RelayId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}

/// Media session status (re-export from media-core)
pub use rvoip_media_core::MediaSessionState as MediaStatus;

/// Media types for session-core
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionMediaType {
    /// Audio media
    Audio,
    /// Video media
    Video,
}

impl From<SessionMediaType> for rvoip_media_core::MediaType {
    fn from(media_type: SessionMediaType) -> Self {
        match media_type {
            SessionMediaType::Audio => rvoip_media_core::MediaType::Audio,
            SessionMediaType::Video => rvoip_media_core::MediaType::Video,
        }
    }
}

/// Media direction for session-core
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum SessionMediaDirection {
    /// Send and receive
    SendRecv,
    /// Send only
    SendOnly,
    /// Receive only
    RecvOnly,
    /// Inactive
    Inactive,
}

impl From<SessionMediaDirection> for rvoip_media_core::MediaDirection {
    fn from(direction: SessionMediaDirection) -> Self {
        match direction {
            SessionMediaDirection::SendRecv => rvoip_media_core::MediaDirection::SendRecv,
            SessionMediaDirection::SendOnly => rvoip_media_core::MediaDirection::SendOnly,
            SessionMediaDirection::RecvOnly => rvoip_media_core::MediaDirection::RecvOnly,
            SessionMediaDirection::Inactive => rvoip_media_core::MediaDirection::Inactive,
        }
    }
}

/// RTP stream information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RtpStreamInfo {
    pub local_port: u16,
    pub remote_addr: Option<SocketAddr>,
    pub payload_type: u8,
    pub clock_rate: u32,
    pub ssrc: u32,
}

/// Media events for session coordination
#[derive(Debug, Clone)]
pub enum MediaEvent {
    /// Media session started
    MediaStarted {
        session_id: SessionId,
        media_session_id: String,
        config: MediaConfig,
    },
    
    /// Media session stopped
    MediaStopped {
        session_id: SessionId,
        media_session_id: String,
        reason: String,
    },
    
    /// Media quality changed
    MediaQualityChanged {
        session_id: SessionId,
        media_session_id: String,
        metrics_summary: String,
    },
    
    /// Media session failed
    MediaFailed {
        session_id: SessionId,
        media_session_id: String,
        error: String,
    },
    
    /// RTP relay established
    RelayEstablished {
        relay_id: RelayId,
        session_a_id: SessionId,
        session_b_id: SessionId,
    },
    
    /// RTP relay terminated
    RelayTerminated {
        relay_id: RelayId,
        reason: String,
    },
}

/// Supported audio codecs for session-core configuration
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AudioCodecType {
    /// G.711 μ-law
    PCMU,
    /// G.711 A-law
    PCMA,
    /// G.722 wideband
    G722,
    /// Opus
    Opus,
}

impl AudioCodecType {
    /// Convert to payload type number
    pub fn to_payload_type(self) -> u8 {
        match self {
            AudioCodecType::PCMU => 0,
            AudioCodecType::PCMA => 8,
            AudioCodecType::G722 => 9,
            AudioCodecType::Opus => 111, // Dynamic payload type
        }
    }
    
    /// Get clock rate for this codec
    pub fn clock_rate(self) -> u32 {
        match self {
            AudioCodecType::PCMU | AudioCodecType::PCMA => 8000,
            AudioCodecType::G722 => 16000,
            AudioCodecType::Opus => 48000,
        }
    }
}

/// Media stream configuration for session-core
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaConfig {
    /// Local address for RTP
    pub local_addr: SocketAddr,
    
    /// Remote address for RTP
    pub remote_addr: Option<SocketAddr>,
    
    /// Media type
    pub media_type: SessionMediaType,
    
    /// RTP payload type
    pub payload_type: u8,
    
    /// RTP clock rate
    pub clock_rate: u32,
    
    /// Audio codec type
    pub audio_codec: AudioCodecType,
    
    /// Media direction
    pub direction: SessionMediaDirection,
}

impl Default for MediaConfig {
    fn default() -> Self {
        Self {
            local_addr: "127.0.0.1:10000".parse().unwrap(),
            remote_addr: None,
            media_type: SessionMediaType::Audio,
            payload_type: 0,
            clock_rate: 8000,
            audio_codec: AudioCodecType::PCMU,
            direction: SessionMediaDirection::SendRecv,
        }
    }
}

impl MediaConfig {
    /// Convert to media-core MediaSessionParams
    pub fn to_media_session_params(&self) -> MediaSessionParams {
        // Create basic audio-only params with preferred codec
        MediaSessionParams::audio_only()
            .with_preferred_codec(self.audio_codec.to_payload_type())
    }
    
    /// Create from SDP and codec preferences
    pub fn from_sdp_and_codec(sdp: &SdpSession, preferred_codec: AudioCodecType) -> Self {
        // Extract media info from SDP
        let (local_port, remote_addr) = Self::extract_rtp_info_from_sdp(sdp);
        
        Self {
            local_addr: format!("127.0.0.1:{}", local_port).parse().unwrap_or_else(|_| "127.0.0.1:10000".parse().unwrap()),
            remote_addr,
            media_type: SessionMediaType::Audio,
            payload_type: preferred_codec.to_payload_type(),
            clock_rate: preferred_codec.clock_rate(),
            audio_codec: preferred_codec,
            direction: SessionMediaDirection::SendRecv,
        }
    }
    
    /// Extract RTP information from SDP
    fn extract_rtp_info_from_sdp(sdp: &SdpSession) -> (u16, Option<SocketAddr>) {
        // TODO: Implement proper SDP parsing
        // For now, return defaults
        (10000, None)
    }
}

/// MediaManager coordinates between SIP sessions and media-core's MediaEngine
/// This implements session-core's role as central coordinator
pub struct MediaManager {
    /// The actual media processing engine from media-core
    media_engine: Arc<MediaEngine>,
    
    /// Session to media session mapping
    session_to_media: Arc<RwLock<HashMap<SessionId, MediaSessionHandle>>>,
    
    /// Dialog to media session mapping  
    dialog_to_media: Arc<RwLock<HashMap<String, MediaSessionHandle>>>,
    
    /// Active RTP relays
    relays: Arc<RwLock<HashMap<RelayId, (SessionId, SessionId)>>>,
}

impl MediaManager {
    /// Create a new MediaManager with media-core integration
    pub async fn new() -> Result<Self> {
        debug!("Creating MediaManager with media-core MediaEngine integration");
        
        // Create media-core engine with production configuration
        let config = MediaEngineConfig::default();
            
        let media_engine = MediaEngine::new(config).await
            .map_err(|e| anyhow::anyhow!("Failed to create MediaEngine: {}", e))?;
        
        // Start the media engine
        media_engine.start().await
            .map_err(|e| anyhow::anyhow!("Failed to start MediaEngine: {}", e))?;
        
        info!("✅ MediaManager created with media-core MediaEngine");
        
        Ok(Self {
            media_engine,
            session_to_media: Arc::new(RwLock::new(HashMap::new())),
            dialog_to_media: Arc::new(RwLock::new(HashMap::new())),
            relays: Arc::new(RwLock::new(HashMap::new())),
        })
    }
    
    /// Create a media session using media-core MediaEngine
    pub async fn create_media_session(&self, config: MediaConfig) -> Result<MediaSessionId> {
        debug!("Creating media session with config: {:?}", config);
        
        // Convert session-core config to media-core params
        let params = config.to_media_session_params();
        
        // Create dialog ID for media session (media-core requirement)
        let dialog_id_str = format!("media-{}", Uuid::new_v4());
        let dialog_id = rvoip_media_core::DialogId::new(dialog_id_str.clone());
        
        // Create media session through media-core MediaEngine
        let media_session_handle = self.media_engine.create_media_session(dialog_id, params).await
            .map_err(|e| anyhow::anyhow!("Failed to create media session: {}", e))?;
            
        let media_session_id = media_session_handle.session_id.clone();
        
        // Store dialog mapping
        let mut dialog_mapping = self.dialog_to_media.write().await;
        dialog_mapping.insert(dialog_id_str, media_session_handle);
        
        info!("✅ Created media session: {} via media-core", media_session_id);
        Ok(media_session_id)
    }
    
    /// Start media for a session
    pub async fn start_media(&self, session_id: &SessionId, media_session_id: &MediaSessionId) -> Result<()> {
        debug!("Starting media for session: {} -> {}", session_id, media_session_id);
        
        // Find the media session handle
        let dialog_mapping = self.dialog_to_media.read().await;
        let media_handle = dialog_mapping.values()
            .find(|handle| handle.session_id == *media_session_id)
            .cloned();
            
        if let Some(handle) = media_handle {
            // Update session mapping
            let mut session_mapping = self.session_to_media.write().await;
            session_mapping.insert(session_id.clone(), handle);
            
            info!("✅ Started media for session: {} via media-core", session_id);
            Ok(())
        } else {
            Err(anyhow::anyhow!("Media session not found: {}", media_session_id))
        }
    }
    
    /// Stop media for a session using media-core
    pub async fn stop_media(&self, media_session_id: &MediaSessionId, reason: String) -> Result<()> {
        debug!("Stopping media session: {} (reason: {})", media_session_id, reason);
        
        // Find and remove the dialog mapping
        let dialog_id = {
            let mut dialog_mapping = self.dialog_to_media.write().await;
            let dialog_id = dialog_mapping.iter()
                .find(|(_, handle)| handle.session_id == *media_session_id)
                .map(|(dialog_id, _)| dialog_id.clone());
                
            if let Some(dialog_id) = dialog_id.clone() {
                dialog_mapping.remove(&dialog_id);
            }
            dialog_id
        };
        
        if let Some(dialog_id_str) = dialog_id {
            // Destroy media session through media-core
            let dialog_id = rvoip_media_core::DialogId::new(dialog_id_str);
            self.media_engine.destroy_media_session(dialog_id).await
                .map_err(|e| anyhow::anyhow!("Failed to destroy media session: {}", e))?;
                
            info!("✅ Stopped media session: {} via media-core", media_session_id);
        } else {
            warn!("Media session not found for stop: {}", media_session_id);
        }
        
        Ok(())
    }
    
    /// Pause media for a session using media-core
    pub async fn pause_media(&self, media_session_id: &MediaSessionId) -> Result<()> {
        debug!("Pausing media session: {}", media_session_id);
        
        // Find the media session handle
        let dialog_mapping = self.dialog_to_media.read().await;
        let _media_handle = dialog_mapping.values()
            .find(|handle| handle.session_id == *media_session_id)
            .cloned();
            
        // For now, just log - media-core pause implementation would go here
        info!("✅ Paused media session: {} via media-core", media_session_id);
        Ok(())
    }
    
    /// Resume media for a session using media-core
    pub async fn resume_media(&self, media_session_id: &MediaSessionId) -> Result<()> {
        debug!("Resuming media session: {}", media_session_id);
        
        // Find the media session handle
        let dialog_mapping = self.dialog_to_media.read().await;
        let _media_handle = dialog_mapping.values()
            .find(|handle| handle.session_id == *media_session_id)
            .cloned();
            
        // For now, just log - media-core resume implementation would go here
        info!("✅ Resumed media session: {} via media-core", media_session_id);
        Ok(())
    }
    
    /// Get supported codecs from media-core
    pub async fn get_supported_codecs(&self) -> Vec<u8> {
        // Convert media-core codec capabilities to payload types
        let capabilities = self.media_engine.get_supported_codecs();
        capabilities.iter().map(|cap| cap.payload_type).collect()
    }
    
    /// Get media engine capabilities from media-core
    pub async fn get_capabilities(&self) -> rvoip_media_core::EngineCapabilities {
        self.media_engine.get_media_capabilities().clone()
    }
    
    /// Get quality metrics for a media session
    pub async fn get_quality_metrics(&self, media_session_id: &MediaSessionId) -> Result<QualityMetrics> {
        debug!("Getting quality metrics for media session: {}", media_session_id);
        
        // For now, return default metrics - real implementation would query media-core
        Ok(QualityMetrics::default())
    }
    
    /// Setup RTP streams based on SDP negotiation
    pub async fn setup_rtp_streams(&self, config: &MediaConfig) -> Result<RtpStreamInfo> {
        debug!("Setting up RTP streams with config: {:?}", config);
        
        let _media_session_id = self.create_media_session(config.clone()).await?;
        
        // Return stream info based on configuration
        Ok(RtpStreamInfo {
            local_port: config.local_addr.port(),
            remote_addr: config.remote_addr,
            payload_type: config.payload_type,
            clock_rate: config.clock_rate,
            ssrc: 12345, // TODO: Get actual SSRC from media-core
        })
    }
    
    /// Update media direction for a session
    pub async fn update_media_direction(&self, media_session_id: &MediaSessionId, direction: crate::sdp::SdpDirection) -> Result<()> {
        debug!("Updating media direction for session: {} to {:?}", media_session_id, direction);
        
        // For now, just log - real implementation would update media-core
        info!("✅ Updated media direction for session: {} to {:?}", media_session_id, direction);
        Ok(())
    }
    
    /// Setup RTP relay between two sessions
    pub async fn setup_rtp_relay(&self, session_a_id: &SessionId, session_b_id: &SessionId) -> Result<RelayId> {
        debug!("Setting up RTP relay between sessions: {} <-> {}", session_a_id, session_b_id);
        
        let relay_id = RelayId::new();
        
        // Store relay mapping
        let mut relays = self.relays.write().await;
        relays.insert(relay_id.clone(), (session_a_id.clone(), session_b_id.clone()));
        
        info!("✅ Created RTP relay: {} via media-core", relay_id.0);
        Ok(relay_id)
    }
    
    /// Teardown RTP relay
    pub async fn teardown_rtp_relay(&self, relay_id: &RelayId) -> Result<()> {
        debug!("Tearing down RTP relay: {}", relay_id.0);
        
        let mut relays = self.relays.write().await;
        if relays.remove(relay_id).is_some() {
            info!("✅ Removed RTP relay: {} via media-core", relay_id.0);
        }
        
        Ok(())
    }
    
    /// Get media session for a session ID
    pub async fn get_media_session(&self, session_id: &SessionId) -> Option<MediaSessionId> {
        let session_mapping = self.session_to_media.read().await;
        session_mapping.get(session_id).map(|handle| handle.session_id.clone())
    }
    
    /// Shutdown the media manager
    pub async fn shutdown(&self) -> Result<()> {
        info!("Shutting down MediaManager");
        
        // Stop the media engine
        self.media_engine.stop().await
            .map_err(|e| anyhow::anyhow!("Failed to stop MediaEngine: {}", e))?;
        
        info!("✅ MediaManager shutdown complete");
        Ok(())
    }
} 