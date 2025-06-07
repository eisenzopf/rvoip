//! Media Types for Session-Core Integration
//!
//! Modern type definitions adapted to the new session-core architecture,
//! providing clean interfaces between SIP signaling and media-core processing.

use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

// Import real media-core types with aliases to avoid conflicts
pub use rvoip_media_core::{
    MediaEngine,
    MediaEngineConfig,
    EngineCapabilities,
    MediaSessionParams,
    MediaSessionHandle,
    MediaSession,
    MediaSessionConfig,
    SessionEvent as MediaCoreSessionEvent,
    Error as MediaCoreError,
    Result as MediaCoreResult,
    relay::{
        MediaSessionController,
        MediaConfig as MediaCoreConfig,
        MediaSessionStatus as MediaCoreSessionStatus,
        MediaSessionInfo as MediaCoreSessionInfo,
        MediaSessionEvent as ControllerEvent,
        DialogId,
        G711PcmuCodec,
        G711PcmaCodec,
    },
};

// Use media-core state enum directly
pub use rvoip_media_core::MediaSessionState;

/// Session identifier for media coordination (mapped to DialogId)
pub type MediaSessionId = DialogId;

/// RTP port number
pub type RtpPort = u16;

/// Media session information (wrapper around media-core types)
#[derive(Debug, Clone)]
pub struct MediaSessionInfo {
    pub session_id: MediaSessionId,
    pub local_sdp: Option<String>,
    pub remote_sdp: Option<String>,
    pub local_rtp_port: Option<RtpPort>,
    pub remote_rtp_port: Option<RtpPort>,
    pub codec: Option<String>,
    pub quality_metrics: Option<QualityMetrics>,
}

impl Default for MediaSessionInfo {
    fn default() -> Self {
        Self {
            session_id: String::new(),
            local_sdp: None,
            remote_sdp: None,
            local_rtp_port: None,
            remote_rtp_port: None,
            codec: None,
            quality_metrics: None,
        }
    }
}

/// Quality metrics for media sessions
#[derive(Debug, Clone)]
pub struct QualityMetrics {
    pub mos_score: Option<f32>,
    pub packet_loss: Option<f32>,
    pub jitter: Option<f32>,
    pub latency: Option<u32>,
}

/// Media capabilities supported by the engine
#[derive(Debug, Clone)]
pub struct MediaCapabilities {
    pub codecs: Vec<CodecInfo>,
    pub max_sessions: usize,
    pub port_range: (RtpPort, RtpPort),
}

/// Codec information
#[derive(Debug, Clone)]
pub struct CodecInfo {
    pub name: String,
    pub payload_type: u8,
    pub sample_rate: u32,
    pub channels: u8,
}

/// Session-core specific media configuration (wrapper)
#[derive(Debug, Clone)]
pub struct MediaConfig {
    pub preferred_codecs: Vec<String>,
    pub port_range: Option<(RtpPort, RtpPort)>,
    pub quality_monitoring: bool,
    pub dtmf_support: bool,
}

impl Default for MediaConfig {
    fn default() -> Self {
        Self {
            preferred_codecs: vec!["PCMU".to_string(), "PCMA".to_string()],
            port_range: Some((10000, 20000)),
            quality_monitoring: true,
            dtmf_support: true,
        }
    }
}

/// Media event types for session coordination
#[derive(Debug, Clone)]
pub enum MediaEvent {
    /// Media session successfully established
    SessionEstablished {
        session_id: MediaSessionId,
        info: MediaSessionInfo,
    },
    
    /// Media session terminated
    SessionTerminated {
        session_id: MediaSessionId,
    },
    
    /// Quality metrics updated
    QualityUpdate {
        session_id: MediaSessionId,
        metrics: QualityMetrics,
    },
    
    /// DTMF tone detected
    DtmfDetected {
        session_id: MediaSessionId,
        tone: char,
        duration: u32,
    },
    
    /// Media error occurred
    Error {
        session_id: MediaSessionId,
        error: String,
    },
}

/// Storage for active media sessions
pub type MediaSessionStorage = Arc<RwLock<HashMap<MediaSessionId, MediaSessionInfo>>>;

/// Real MediaSessionController adapter - this is our primary media integration
pub type SessionCoreMediaEngine = MediaSessionController;

/// Conversion between session-core MediaSessionInfo and media-core MediaSessionInfo
impl From<MediaCoreSessionInfo> for MediaSessionInfo {
    fn from(core_info: MediaCoreSessionInfo) -> Self {
        Self {
            session_id: core_info.dialog_id,
            local_sdp: None, // SDP should come from actual SDP generation, not hardcoded
            remote_sdp: None, // SDP should come from actual negotiation, not hardcoded
            local_rtp_port: core_info.rtp_port,
            remote_rtp_port: core_info.config.remote_addr.map(|addr| addr.port()),
            codec: core_info.config.preferred_codec.or_else(|| Some("PCMU".to_string())),
            quality_metrics: None, // TODO: Convert from stats if available
        }
    }
}

/// Helper function to convert session-core MediaConfig to media-core MediaConfig
pub fn convert_to_media_core_config(
    config: &MediaConfig,
    local_addr: std::net::SocketAddr,
    remote_addr: Option<std::net::SocketAddr>,
) -> MediaCoreConfig {
    MediaCoreConfig {
        local_addr,
        remote_addr,
        preferred_codec: config.preferred_codecs.first().cloned(),
        parameters: HashMap::new(),
    }
} 