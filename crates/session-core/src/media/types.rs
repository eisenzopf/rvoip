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

// Import RTP and performance types from media-core (proper separation of concerns)
use rvoip_media_core::prelude::RtpPacket;
use rvoip_media_core::performance::pool::PoolStats;
use rvoip_media_core::relay::controller::codec_fallback::FallbackMode;

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
            session_id: DialogId::new(""),
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

/// Media configuration for session management
#[derive(Debug, Clone)]
pub struct MediaConfig {
    /// Preferred codecs in priority order
    pub preferred_codecs: Vec<String>,
    
    /// RTP port range for media sessions
    pub port_range: Option<(RtpPort, RtpPort)>,
    
    /// Enable quality monitoring and metrics collection
    pub quality_monitoring: bool,
    
    /// Enable DTMF support
    pub dtmf_support: bool,
    
    /// Enable echo cancellation
    pub echo_cancellation: bool,
    
    /// Enable noise suppression
    pub noise_suppression: bool,
    
    /// Enable automatic gain control
    pub auto_gain_control: bool,
    
    /// Path to music-on-hold WAV file
    /// If None, silence will be sent during hold
    pub music_on_hold_path: Option<std::path::PathBuf>,
    
    /// Maximum bandwidth in kbps
    pub max_bandwidth_kbps: Option<u32>,
    
    /// Preferred packetization time
    pub preferred_ptime: Option<u8>,
    
    /// Custom SDP attributes for advanced use cases
    pub custom_sdp_attributes: std::collections::HashMap<String, String>,
}

impl Default for MediaConfig {
    fn default() -> Self {
        Self {
            preferred_codecs: vec!["PCMU".to_string(), "PCMA".to_string()],
            port_range: Some((10000, 20000)),
            quality_monitoring: true,
            dtmf_support: true,
            echo_cancellation: true,
            noise_suppression: true,
            auto_gain_control: true,
            music_on_hold_path: None,
            max_bandwidth_kbps: None,
            preferred_ptime: Some(20),
            custom_sdp_attributes: std::collections::HashMap::new(),
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
    
    /// RTP packet processed with zero-copy
    RtpPacketProcessed {
        session_id: MediaSessionId,
        processing_type: RtpProcessingType,
        performance_metrics: RtpProcessingMetrics,
    },
    
    /// Rtp processing mode changed
    RtpProcessingModeChanged {
        session_id: MediaSessionId,
        old_mode: RtpProcessingMode,
        new_mode: RtpProcessingMode,
    },
    
    /// Rtp processing error
    RtpProcessingError {
        session_id: MediaSessionId,
        error: String,
        fallback_applied: bool,
    },
    
    /// Rtp buffer pool statistics update
    RtpBufferPoolUpdate {
        stats: RtpBufferPoolStats,
    },
}

/// Configuration for zero-copy RTP processing per session
#[derive(Debug, Clone)]
pub struct ZeroCopyConfig {
    /// Whether zero-copy processing is enabled
    pub enabled: bool,
    /// Fallback to traditional processing on errors
    pub fallback_enabled: bool,
    /// Performance monitoring enabled
    pub monitoring_enabled: bool,
}

impl Default for ZeroCopyConfig {
    fn default() -> Self {
        Self {
            enabled: true,
            fallback_enabled: true,
            monitoring_enabled: true,
        }
    }
}

/// RTP processing performance metrics
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RtpProcessingMetrics {
    pub zero_copy_packets_processed: u64,
    pub traditional_packets_processed: u64,
    pub fallback_events: u64,
    pub average_processing_time_zero_copy: f64, // microseconds
    pub average_processing_time_traditional: f64, // microseconds
    pub allocation_reduction_percentage: f32,
}

/// RTP processing types for events
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum RtpProcessingType {
    ZeroCopy,
    Traditional,
    Fallback,
}

/// RTP processing modes
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub enum RtpProcessingMode {
    ZeroCopyPreferred,
    TraditionalOnly,
    Adaptive,
}

/// RTP buffer pool statistics wrapper
#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct RtpBufferPoolStats {
    pub total_buffers: usize,
    pub available_buffers: usize,
    pub in_use_buffers: usize,
    pub allocation_rate: f64, // buffers per second
    pub efficiency_percentage: f32,
}

impl From<PoolStats> for RtpBufferPoolStats {
    fn from(stats: PoolStats) -> Self {
        Self {
            total_buffers: stats.total_allocated,
            available_buffers: stats.available,
            in_use_buffers: stats.pool_size, // Use pool_size as approximation for in_use
            allocation_rate: 0.0, // TODO: Calculate from stats
            efficiency_percentage: if stats.total_allocated > 0 {
                (stats.pool_size as f32 / stats.total_allocated as f32) * 100.0
            } else {
                0.0
            },
        }
    }
}

/// Codec processing statistics for monitoring detection and fallback
#[derive(Debug, Clone)]
pub struct CodecProcessingStats {
    /// Session identifier
    pub session_id: super::super::api::types::SessionId,
    /// Expected codec based on SDP negotiation
    pub expected_codec: Option<String>,
    /// Detected codec from actual RTP packets
    pub detected_codec: Option<String>,
    /// Confidence level in codec detection (0.0 to 1.0)
    pub detection_confidence: f32,
    /// Number of packets analyzed for detection
    pub packets_analyzed: u64,
    /// Current fallback mode
    pub fallback_mode: FallbackMode,
    /// Fallback processing efficiency (0.0 to 1.0)
    pub fallback_efficiency: f32,
    /// Whether transcoding is currently active
    pub transcoding_active: bool,
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