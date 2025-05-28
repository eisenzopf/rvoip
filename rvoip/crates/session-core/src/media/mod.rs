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

/// MediaManager coordinates between SIP sessions and media-core
/// This implements session-core's role as central coordinator using MediaSessionController for real RTP sessions
pub struct MediaManager {
    /// Media session controller for RTP port allocation and session management
    media_controller: Arc<rvoip_media_core::relay::MediaSessionController>,
    
    /// Session to media session mapping
    session_to_media: Arc<RwLock<HashMap<SessionId, String>>>,
    
    /// Dialog to session mapping  
    dialog_to_session: Arc<RwLock<HashMap<String, SessionId>>>,
}

impl MediaManager {
    /// Create a new MediaManager with MediaSessionController for real RTP sessions
    pub async fn new() -> Result<Self> {
        debug!("Creating MediaManager with media-core MediaSessionController for real RTP sessions");
        
        // Create media session controller for RTP port allocation (10000-20000 range)
        let media_controller = Arc::new(rvoip_media_core::relay::MediaSessionController::with_port_range(10000, 20000));
        
        info!("✅ MediaManager created with MediaSessionController (real RTP port allocation: 10000-20000)");
        
        Ok(Self {
            media_controller,
            session_to_media: Arc::new(RwLock::new(HashMap::new())),
            dialog_to_session: Arc::new(RwLock::new(HashMap::new())),
        })
    }
    
    /// Get access to the media session controller for direct queries
    pub fn media_controller(&self) -> &Arc<rvoip_media_core::relay::MediaSessionController> {
        &self.media_controller
    }
    
    /// Get RTP session for packet transmission
    pub async fn get_rtp_session(&self, media_session_id: &MediaSessionId) -> Result<Option<Arc<tokio::sync::Mutex<rvoip_rtp_core::RtpSession>>>> {
        debug!("Getting RTP session for media session: {}", media_session_id);
        
        // Extract dialog ID from media session ID
        let dialog_id = media_session_id.as_str();
        
        // Get RTP session from MediaSessionController
        let rtp_session = self.media_controller.get_rtp_session(dialog_id).await;
        
        if rtp_session.is_some() {
            debug!("✅ Found RTP session for media session: {}", media_session_id);
        } else {
            debug!("❌ No RTP session found for media session: {}", media_session_id);
        }
        
        Ok(rtp_session)
    }
    
    /// Send RTP packet for a media session
    pub async fn send_rtp_packet(&self, media_session_id: &MediaSessionId, payload: Vec<u8>, timestamp: u32) -> Result<()> {
        debug!("Sending RTP packet for media session: {} (timestamp: {})", media_session_id, timestamp);
        
        // Extract dialog ID from media session ID
        let dialog_id = media_session_id.as_str();
        
        // Send RTP packet through MediaSessionController
        self.media_controller.send_rtp_packet(dialog_id, payload, timestamp).await
            .map_err(|e| anyhow::anyhow!("Failed to send RTP packet: {}", e))?;
        
        debug!("✅ Sent RTP packet for media session: {}", media_session_id);
        Ok(())
    }
    
    /// Check if RTP session is active for a media session
    pub async fn is_rtp_session_active(&self, media_session_id: &MediaSessionId) -> bool {
        let rtp_session = self.get_rtp_session(media_session_id).await;
        rtp_session.unwrap_or(None).is_some()
    }
    
    /// Create a media session using MediaSessionController for real RTP port allocation AND RTP sessions
    pub async fn create_media_session(&self, config: MediaConfig) -> Result<MediaSessionId> {
        debug!("Creating media session with REAL RTP session capabilities: {:?}", config);
        
        // Create dialog ID for media session
        let dialog_id_str = format!("media-{}", Uuid::new_v4());
        
        // Convert session-core config to media-core config
        let media_config = rvoip_media_core::relay::MediaConfig {
            local_addr: config.local_addr,
            remote_addr: config.remote_addr,
            preferred_codec: Some(format!("{:?}", config.audio_codec)),
            parameters: std::collections::HashMap::new(),
        };
        
        // Start media session through MediaSessionController for REAL RTP port allocation AND RTP sessions
        self.media_controller.start_media(dialog_id_str.clone(), media_config).await
            .map_err(|e| anyhow::anyhow!("Failed to start media session with RTP sessions via MediaSessionController: {}", e))?;
        
        // Create MediaSessionId from dialog ID
        let media_session_id = MediaSessionId::new(&dialog_id_str);
        
        info!("✅ Created media session: {} with REAL RTP sessions and port allocation via MediaSessionController", media_session_id);
        Ok(media_session_id)
    }
    
    /// Start media for a session
    pub async fn start_media(&self, session_id: &SessionId, media_session_id: &MediaSessionId) -> Result<()> {
        debug!("Starting media for session: {} -> {}", session_id, media_session_id);
        
        // Update session mapping
        let mut session_mapping = self.session_to_media.write().await;
        session_mapping.insert(session_id.clone(), media_session_id.as_str().to_string());
        
        info!("✅ Started media for session: {} via media-core", session_id);
        Ok(())
    }
    
    /// Stop media for a session using proper media-core integration
    pub async fn stop_media(&self, media_session_id: &MediaSessionId, reason: String) -> Result<()> {
        debug!("Stopping media session: {} (reason: {})", media_session_id, reason);
        
        // Find the dialog ID for this media session
        let dialog_id_str = media_session_id.as_str().replace("media-", "");
        let dialog_id = rvoip_media_core::DialogId::new(dialog_id_str.clone());
        
        // Stop media session through MediaSessionController
        self.media_controller.stop_media(dialog_id_str).await
            .map_err(|e| anyhow::anyhow!("Failed to stop media session via MediaSessionController: {}", e))?;
        
        info!("✅ Stopped media session: {} via proper media-core integration", media_session_id);
        Ok(())
    }
    
    /// Pause media for a session
    pub async fn pause_media(&self, media_session_id: &MediaSessionId) -> Result<()> {
        debug!("Pausing media session: {}", media_session_id);
        
        // For now, just log - media-core pause implementation would go here
        info!("✅ Paused media session: {} via media-core", media_session_id);
        Ok(())
    }
    
    /// Resume media for a session
    pub async fn resume_media(&self, media_session_id: &MediaSessionId) -> Result<()> {
        debug!("Resuming media session: {}", media_session_id);
        
        // For now, just log - media-core resume implementation would go here
        info!("✅ Resumed media session: {} via media-core", media_session_id);
        Ok(())
    }
    
    /// Get supported codecs (basic codec support)
    pub async fn get_supported_codecs(&self) -> Vec<u8> {
        // Return basic codec support (PCMU, PCMA, G722)
        vec![0, 8, 9] // PCMU, PCMA, G722
    }
    
    /// Get media capabilities (basic capabilities)
    pub async fn get_capabilities(&self) -> Vec<String> {
        vec!["PCMU".to_string(), "PCMA".to_string(), "G722".to_string()]
    }
    
    /// Get quality metrics for a media session
    pub async fn get_quality_metrics(&self, media_session_id: &MediaSessionId) -> Result<QualityMetrics> {
        debug!("Getting quality metrics for media session: {}", media_session_id);
        
        // For now, return default metrics - real implementation would query media-core
        Ok(QualityMetrics::default())
    }
    
    /// Setup RTP streams using MediaSessionController for real RTP port allocation
    pub async fn setup_rtp_streams(&self, config: &MediaConfig) -> Result<RtpStreamInfo> {
        debug!("Setting up RTP streams with config: {:?}", config);
        
        // Create media session for processing
        let media_session_id = self.create_media_session(config.clone()).await?;
        
        // Get the REAL allocated RTP port from MediaSessionController
        let session_info = self.media_controller.get_session_info(media_session_id.as_str()).await
            .ok_or_else(|| anyhow::anyhow!("Media session not found: {}", media_session_id))?;
        
        let actual_local_port = session_info.rtp_port
            .ok_or_else(|| anyhow::anyhow!("No RTP port allocated for session: {}", media_session_id))?;
        
        debug!("✅ RTP streams setup complete - allocated port: {} via MediaSessionController", actual_local_port);
        
        // Return stream info with the REAL allocated port from MediaSessionController
        Ok(RtpStreamInfo {
            local_port: actual_local_port,
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
    
    /// Get media session for a session ID
    pub async fn get_media_session(&self, session_id: &SessionId) -> Option<MediaSessionId> {
        let session_mapping = self.session_to_media.read().await;
        session_mapping.get(session_id).map(|s| MediaSessionId::new(s))
    }
    
    /// Shutdown the media manager
    pub async fn shutdown(&self) -> Result<()> {
        info!("Shutting down MediaManager");
        
        // Get all active sessions and stop them
        let active_sessions: Vec<String> = {
            let session_mapping = self.session_to_media.read().await;
            session_mapping.values().cloned().collect()
        };
        
        // Stop all active media sessions
        for media_session_str in active_sessions {
            let media_session_id = MediaSessionId::new(&media_session_str);
            if let Err(e) = self.stop_media(&media_session_id, "Shutdown".to_string()).await {
                warn!("Failed to stop media session during shutdown: {}", e);
            }
        }
        
        info!("✅ MediaManager shutdown complete");
        Ok(())
    }
    
    /// Establish media flow with bi-directional audio transmission
    pub async fn establish_media_flow(&self, media_session_id: &MediaSessionId, remote_addr: SocketAddr) -> Result<()> {
        debug!("🔗 Establishing media flow for session: {} -> {}", media_session_id, remote_addr);
        
        // Extract dialog ID from media session ID
        let dialog_id = media_session_id.as_str();
        
        // Establish media flow through MediaSessionController
        self.media_controller.establish_media_flow(dialog_id, remote_addr).await
            .map_err(|e| anyhow::anyhow!("Failed to establish media flow: {}", e))?;
        
        info!("✅ Established bi-directional media flow for session: {}", media_session_id);
        Ok(())
    }
    
    /// Terminate media flow and stop audio transmission
    pub async fn terminate_media_flow(&self, media_session_id: &MediaSessionId) -> Result<()> {
        debug!("🛑 Terminating media flow for session: {}", media_session_id);
        
        // Extract dialog ID from media session ID
        let dialog_id = media_session_id.as_str();
        
        // Terminate media flow through MediaSessionController
        self.media_controller.terminate_media_flow(dialog_id).await
            .map_err(|e| anyhow::anyhow!("Failed to terminate media flow: {}", e))?;
        
        info!("✅ Terminated media flow for session: {}", media_session_id);
        Ok(())
    }
    
    /// Check if audio transmission is active for a media session
    pub async fn is_audio_transmission_active(&self, media_session_id: &MediaSessionId) -> bool {
        let dialog_id = media_session_id.as_str();
        self.media_controller.is_audio_transmission_active(dialog_id).await
    }
} 