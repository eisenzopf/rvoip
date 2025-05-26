use std::net::SocketAddr;
use std::sync::Arc;
use std::collections::HashMap;
use std::any::Any;
use tokio::sync::{Mutex, mpsc, RwLock};
use anyhow::Result;
use uuid::Uuid;
use serde::{Serialize, Deserialize};
use tracing::{info, debug, error};

// Import media-core components
use rvoip_media_core::prelude::*;

use crate::session::SessionId;
use crate::dialog::DialogId;
use crate::sdp::SessionDescription;

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

/// Media quality metrics (re-export from media-core)
pub use rvoip_media_core::QualityMetrics;

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
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum MediaEvent {
    /// Media session started
    MediaStarted {
        session_id: SessionId,
        media_session_id: MediaSessionId,
        config: MediaConfig,
    },
    
    /// Media session stopped
    MediaStopped {
        session_id: SessionId,
        media_session_id: MediaSessionId,
        reason: String,
    },
    
    /// Media quality changed
    MediaQualityChanged {
        session_id: SessionId,
        media_session_id: MediaSessionId,
        metrics: QualityMetrics,
    },
    
    /// Media session failed
    MediaFailed {
        session_id: SessionId,
        media_session_id: MediaSessionId,
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

/// Supported media types (re-export from media-core)
pub use rvoip_media_core::MediaType;

/// Supported audio codecs
#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
pub enum AudioCodecType {
    /// G.711 μ-law
    PCMU,
    /// G.711 A-law
    PCMA,
}

/// Media stream configuration
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaConfig {
    /// Local address for RTP
    pub local_addr: SocketAddr,
    
    /// Remote address for RTP
    pub remote_addr: Option<SocketAddr>,
    
    /// Media type
    pub media_type: MediaType,
    
    /// RTP payload type
    pub payload_type: u8,
    
    /// RTP clock rate
    pub clock_rate: u32,
    
    /// Audio codec type
    pub audio_codec: AudioCodecType,
}

impl Default for MediaConfig {
    fn default() -> Self {
        Self {
            local_addr: "127.0.0.1:10000".parse().unwrap(),
            remote_addr: None,
            media_type: MediaType::Audio,
            payload_type: 0,
            clock_rate: 8000,
            audio_codec: AudioCodecType::PCMU,
        }
    }
}

/// MediaManager coordinates between SIP sessions and media-core's MediaEngine
/// This is the bridge that implements session-core's role as central coordinator
pub struct MediaManager {
    /// The actual media processing engine from media-core
    media_engine: Arc<MediaEngine>,
    
    /// Session to media session mapping
    session_to_media: Arc<RwLock<HashMap<SessionId, MediaSessionId>>>,
    
    /// Dialog to media session mapping
    dialog_to_media: Arc<RwLock<HashMap<DialogId, MediaSessionId>>>,
    
    /// Active RTP relays
    relays: Arc<RwLock<HashMap<RelayId, (SessionId, SessionId)>>>,
}

impl MediaManager {
    /// Create a new MediaManager with media-core integration
    pub async fn new() -> Result<Self> {
        debug!("Creating MediaManager with media-core integration");
        
        // Create media-core engine with default configuration
        let config = MediaEngineConfig::default();
        let media_engine = Arc::new(MediaEngine::new(config).await?);
        
        // Start the media engine
        media_engine.start().await?;
        
        Ok(Self {
            media_engine,
            session_to_media: Arc::new(RwLock::new(HashMap::new())),
            dialog_to_media: Arc::new(RwLock::new(HashMap::new())),
            relays: Arc::new(RwLock::new(HashMap::new())),
        })
    }
    
    /// Create a media session using media-core
    pub async fn create_media_session(&self, config: MediaConfig) -> Result<MediaSessionId> {
        debug!("Creating media session with config: {:?}", config);
        
        // Convert our config to media-core's MediaSessionParams
        let payload_type = match config.audio_codec {
            AudioCodecType::PCMU => payload_types::PCMU,
            AudioCodecType::PCMA => payload_types::PCMA,
        };
        
        let params = MediaSessionParams::audio_only()
            .with_preferred_codec(payload_type);
        
        // Create dialog ID for media session
        let dialog_id = DialogId::new(&format!("media-{}", Uuid::new_v4()));
        
        // Create media session through media-core
        let media_session_handle = self.media_engine.create_media_session(dialog_id.clone(), params).await?;
        let media_session_id = media_session_handle.session_id();
        
        // Store dialog mapping
        let mut dialog_mapping = self.dialog_to_media.write().await;
        dialog_mapping.insert(dialog_id, media_session_id.clone());
        
        info!("Created media session: {}", media_session_id);
        Ok(media_session_id)
    }
    
    /// Start media for a session
    pub async fn start_media(&self, session_id: &SessionId, media_session_id: &MediaSessionId) -> Result<()> {
        debug!("Starting media for session: {} -> {}", session_id, media_session_id);
        
        // Update session mapping
        let mut session_mapping = self.session_to_media.write().await;
        session_mapping.insert(session_id.clone(), media_session_id.clone());
        
        info!("Started media for session: {}", session_id);
        Ok(())
    }
    
    /// Stop media for a session
    pub async fn stop_media(&self, media_session_id: &MediaSessionId, reason: String) -> Result<()> {
        debug!("Stopping media session: {} (reason: {})", media_session_id, reason);
        
        // Find the dialog ID for this media session
        let dialog_mapping = self.dialog_to_media.read().await;
        let dialog_id = dialog_mapping.iter()
            .find(|(_, &ref id)| id == media_session_id)
            .map(|(dialog_id, _)| dialog_id.clone());
        
        if let Some(dialog_id) = dialog_id {
            // Destroy media session through media-core
            self.media_engine.destroy_media_session(dialog_id).await?;
            info!("Stopped media session: {}", media_session_id);
        } else {
            error!("Media session not found: {}", media_session_id);
        }
        
        Ok(())
    }
    
    /// Setup RTP streams based on SDP negotiation
    pub async fn setup_rtp_streams(&self, config: &MediaConfig) -> Result<RtpStreamInfo> {
        debug!("Setting up RTP streams with config: {:?}", config);
        
        let media_session_id = self.create_media_session(config.clone()).await?;
        
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
    pub async fn update_media_direction(&self, media_session_id: &MediaSessionId, _direction: crate::sdp::SdpDirection) -> Result<()> {
        debug!("Updating media direction for session: {}", media_session_id);
        // TODO: Implement media direction updates through media-core
        Ok(())
    }
    
    /// Setup RTP relay between two sessions
    pub async fn setup_rtp_relay(&self, session_a_id: &SessionId, session_b_id: &SessionId) -> Result<RelayId> {
        debug!("Setting up RTP relay between sessions: {} <-> {}", session_a_id, session_b_id);
        
        let relay_id = RelayId::new();
        
        // Store relay mapping
        let mut relays = self.relays.write().await;
        relays.insert(relay_id.clone(), (session_a_id.clone(), session_b_id.clone()));
        
        info!("Created RTP relay: {}", relay_id.0);
        Ok(relay_id)
    }
    
    /// Teardown RTP relay
    pub async fn teardown_rtp_relay(&self, relay_id: &RelayId) -> Result<()> {
        debug!("Tearing down RTP relay: {}", relay_id.0);
        
        let mut relays = self.relays.write().await;
        if relays.remove(relay_id).is_some() {
            info!("Removed RTP relay: {}", relay_id.0);
        }
        
        Ok(())
    }
    
    /// Get media session for a session ID
    pub async fn get_media_session(&self, session_id: &SessionId) -> Option<MediaSessionId> {
        let session_mapping = self.session_to_media.read().await;
        session_mapping.get(session_id).cloned()
    }
    
    /// Get media session information
    pub async fn get_media_session_info(&self, media_session_id: &MediaSessionId) -> Option<Arc<dyn std::any::Any + Send + Sync>> {
        // TODO: Get actual media session info from media-core
        None
    }
    
    /// Pause media for a session
    pub async fn pause_media(&self, media_session_id: &MediaSessionId) -> Result<()> {
        debug!("Pausing media session: {}", media_session_id);
        // TODO: Implement pause through media-core
        info!("Media session {} paused", media_session_id);
        Ok(())
    }
    
    /// Resume media for a session
    pub async fn resume_media(&self, media_session_id: &MediaSessionId) -> Result<()> {
        debug!("Resuming media session: {}", media_session_id);
        // TODO: Implement resume through media-core
        info!("Media session {} resumed", media_session_id);
        Ok(())
    }
    
    /// Get supported codecs from media-core
    pub async fn get_supported_codecs(&self) -> Vec<PayloadType> {
        self.media_engine.get_supported_codecs()
    }
    
    /// Get media engine capabilities
    pub async fn get_capabilities(&self) -> EngineCapabilities {
        self.media_engine.get_capabilities()
    }
    
    /// Shutdown the media manager
    pub async fn shutdown(&self) -> Result<()> {
        info!("Shutting down MediaManager");
        
        // Stop the media engine
        self.media_engine.stop().await?;
        
        Ok(())
    }
} 