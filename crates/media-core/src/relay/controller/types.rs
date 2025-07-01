//! Type definitions for the MediaSessionController
//!
//! This module contains all the type definitions used by the MediaSessionController
//! and its sub-modules.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::{RwLock, mpsc};

use crate::types::{DialogId, AudioFrame};
use crate::processing::audio::{
    AdvancedVoiceActivityDetector, AdvancedVadConfig,
    AdvancedAutomaticGainControl, AdvancedAgcConfig,
    AdvancedAcousticEchoCanceller, AdvancedAecConfig,
};
use crate::performance::{
    metrics::PerformanceMetrics,
    pool::AudioFramePool,
    simd::SimdProcessor,
};
use rvoip_rtp_core::{RtpSession, session::RtpSessionStats};
use super::super::RelayStats;

/// Media configuration for a session
#[derive(Debug, Clone)]
pub struct MediaConfig {
    /// Local RTP address
    pub local_addr: SocketAddr,
    /// Remote RTP address (if known)
    pub remote_addr: Option<SocketAddr>,
    /// Preferred codec (for future implementation)
    pub preferred_codec: Option<String>,
    /// Additional media parameters
    pub parameters: HashMap<String, String>,
}

/// Media session status
#[derive(Debug, Clone, PartialEq)]
pub enum MediaSessionStatus {
    /// Session is being created
    Creating,
    /// Session is active and relaying media
    Active,
    /// Session is on hold
    OnHold,
    /// Session has ended
    Ended,
    /// Session failed
    Failed(String),
}

/// Information about an active media session
#[derive(Debug, Clone)]
pub struct MediaSessionInfo {
    /// Dialog ID this session is associated with
    pub dialog_id: DialogId,
    /// Media relay session IDs (if this is a relay session)
    pub relay_session_ids: Option<(String, String)>,
    /// Current status
    pub status: MediaSessionStatus,
    /// Media configuration
    pub config: MediaConfig,
    /// RTP port allocated for this session
    pub rtp_port: Option<u16>,
    /// Session statistics
    pub stats: Option<RelayStats>,
    /// RTP/RTCP statistics (if available)
    pub rtp_stats: Option<RtpSessionStats>,
    /// Last statistics update time
    pub stats_updated_at: Option<Instant>,
    /// Creation time
    pub created_at: Instant,
}

impl Default for MediaSessionInfo {
    fn default() -> Self {
        Self {
            dialog_id: DialogId::new(""),
            relay_session_ids: None,
            status: MediaSessionStatus::Creating,
            config: MediaConfig {
                local_addr: SocketAddr::from(([0, 0, 0, 0], 0)),
                remote_addr: None,
                preferred_codec: None,
                parameters: HashMap::new(),
            },
            rtp_port: None,
            stats: None,
            rtp_stats: None,
            stats_updated_at: None,
            created_at: Instant::now(),
        }
    }
}

/// Events emitted by the media session controller
#[derive(Debug, Clone)]
pub enum MediaSessionEvent {
    /// Media session created
    SessionCreated {
        dialog_id: DialogId,
        session_id: DialogId,
    },
    /// Media session destroyed
    SessionDestroyed {
        dialog_id: DialogId,
        session_id: DialogId,
    },
    /// Media session failed
    SessionFailed {
        dialog_id: DialogId,
        error: String,
    },
    /// Remote address updated
    RemoteAddressUpdated {
        dialog_id: DialogId,
        remote_addr: SocketAddr,
    },
    /// Statistics updated
    StatisticsUpdated {
        dialog_id: DialogId,
        stats: crate::types::MediaStatistics,
    },
    /// Quality degradation detected
    QualityDegraded {
        dialog_id: DialogId,
        metrics: crate::types::QualityMetrics,
        reason: String,
    },
}

/// Configuration for advanced processors in a session
#[derive(Debug, Clone)]
pub struct AdvancedProcessorConfig {
    /// Enable advanced VAD
    pub enable_advanced_vad: bool,
    /// Advanced VAD configuration
    pub vad_config: AdvancedVadConfig,
    /// Enable advanced AGC
    pub enable_advanced_agc: bool,
    /// Advanced AGC configuration
    pub agc_config: AdvancedAgcConfig,
    /// Enable advanced AEC
    pub enable_advanced_aec: bool,
    /// Advanced AEC configuration
    pub aec_config: AdvancedAecConfig,
    /// Enable SIMD optimizations
    pub enable_simd: bool,
    /// Frame pool size for this session
    pub frame_pool_size: usize,
    /// Sample rate for processing
    pub sample_rate: u32,
}

impl Default for AdvancedProcessorConfig {
    fn default() -> Self {
        Self {
            enable_advanced_vad: false,
            vad_config: AdvancedVadConfig::default(),
            enable_advanced_agc: false,
            agc_config: AdvancedAgcConfig::default(),
            enable_advanced_aec: false,
            aec_config: AdvancedAecConfig::default(),
            enable_simd: true,
            frame_pool_size: 16,
            sample_rate: 8000,
        }
    }
}

/// Advanced processor set for v2 processors per session
#[derive(Debug)]
pub struct AdvancedProcessorSet {
    /// Advanced voice activity detector (v2)
    pub vad: Option<Arc<RwLock<AdvancedVoiceActivityDetector>>>,
    /// Advanced automatic gain control (v2)
    pub agc: Option<Arc<RwLock<AdvancedAutomaticGainControl>>>,
    /// Advanced acoustic echo canceller (v2)
    pub aec: Option<Arc<RwLock<AdvancedAcousticEchoCanceller>>>,
    /// Session-specific frame pool (shared reference)
    pub frame_pool: Arc<AudioFramePool>,
    /// SIMD processor for this session
    pub simd_processor: SimdProcessor,
    /// Performance metrics for this session
    pub metrics: Arc<RwLock<PerformanceMetrics>>,
    /// Configuration used to create these processors
    pub config: AdvancedProcessorConfig,
}

/// RTP session wrapper for MediaSessionController
pub(super) struct RtpSessionWrapper {
    /// The actual RTP session
    pub session: Arc<tokio::sync::Mutex<RtpSession>>,
    /// Local RTP address
    pub local_addr: SocketAddr,
    /// Remote RTP address (if known)
    pub remote_addr: Option<SocketAddr>,
    /// Session creation time
    pub created_at: Instant,
    /// Audio transmitter for outgoing audio
    pub audio_transmitter: Option<super::audio_generation::AudioTransmitter>,
    /// Whether audio transmission is enabled
    pub transmission_enabled: bool,
} 