//! Type definitions for the MediaSessionController
//!
//! This module contains all the type definitions used by the MediaSessionController
//! and its sub-modules.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::Instant;

use crate::performance::{pool::AudioFramePool, simd::SimdProcessor};
use crate::processing::audio::{
    AdvancedAcousticEchoCanceller, AdvancedAecConfig, AdvancedAgcConfig,
    AdvancedAutomaticGainControl, AdvancedVadConfig, AdvancedVoiceActivityDetector,
};
use crate::types::DialogId;
use rvoip_rtp_core::{session::RtpSessionStats, RtpSession};

/// `MediaConfig::parameters` key carrying the negotiated RTP payload type.
pub const RTP_PAYLOAD_TYPE_PARAMETER: &str = "rtp_payload_type";
/// `MediaConfig::parameters` key carrying the negotiated RTP clock rate.
pub const RTP_CLOCK_RATE_PARAMETER: &str = "rtp_clock_rate";
/// `MediaConfig::parameters` key carrying the negotiated audio channel count.
pub const AUDIO_CHANNELS_PARAMETER: &str = "audio_channels";
/// `MediaConfig::parameters` key carrying the negotiated `a=fmtp` parameters.
///
/// Codec-agnostic and deliberately unparsed. For AMR these select the wire
/// framing itself, which makes them part of a session's media identity rather
/// than a tuning detail — see [`super::bridge`].
pub const NEGOTIATED_FMTP_PARAMETER: &str = "negotiated_fmtp";

/// Media configuration for a session
#[derive(Debug, Clone, PartialEq, Eq)]
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

impl MediaConfig {
    /// Attach the exact negotiated RTP identity to a media configuration.
    ///
    /// Keeping these values in the existing parameter map preserves source
    /// compatibility for callers that construct `MediaConfig` with a struct
    /// literal while allowing dynamic codecs such as Opus to use any payload
    /// type selected by SDP.
    pub fn with_negotiated_audio_codec(
        mut self,
        codec: impl Into<String>,
        payload_type: u8,
        clock_rate: u32,
        channels: u8,
    ) -> Self {
        self.preferred_codec = Some(codec.into());
        self.parameters.insert(
            RTP_PAYLOAD_TYPE_PARAMETER.to_string(),
            payload_type.to_string(),
        );
        self.parameters
            .insert(RTP_CLOCK_RATE_PARAMETER.to_string(), clock_rate.to_string());
        self.parameters
            .insert(AUDIO_CHANNELS_PARAMETER.to_string(), channels.to_string());
        self
    }

    /// Record the negotiated `a=fmtp` parameter string, or clear it.
    ///
    /// `None` means the peer sent no usable `a=fmtp` line, which is a
    /// statement rather than an absence of one: for AMR it selects every RFC
    /// 4867 default. It therefore has to *remove* any value a previous
    /// negotiation left behind. A re-INVITE from octet-aligned AMR to
    /// bandwidth-efficient AMR — or to any codec at all — reaches this with
    /// `None`, and callers seed the new configuration from the old one, so
    /// skipping the write would leave the stale string in place and make the
    /// bridge's framing guard refuse a pair that is in fact compatible.
    #[must_use]
    pub fn with_negotiated_fmtp(mut self, fmtp: Option<&str>) -> Self {
        match fmtp {
            Some(fmtp) => {
                self.parameters
                    .insert(NEGOTIATED_FMTP_PARAMETER.to_string(), fmtp.to_string());
            }
            None => {
                self.parameters.remove(NEGOTIATED_FMTP_PARAMETER);
            }
        }
        self
    }
}

/// Exact audio codec parameters used by one media dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct NegotiatedAudioCodec {
    /// Canonical codec name.
    pub name: String,
    /// Negotiated RTP payload type.
    pub payload_type: u8,
    /// RTP clock and PCM sample rate in hertz.
    pub clock_rate: u32,
    /// Number of interleaved PCM channels.
    pub channels: u8,
    /// The negotiated `a=fmtp` line for this codec, when there was one.
    ///
    /// Carried because for AMR it is not decoration: `octet-align` selects the
    /// payload's bit layout, so a session that lost it would build a framing
    /// the peer cannot parse. Every other codec here ignores it.
    pub fmtp: Option<String>,
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
    /// Current status
    pub status: MediaSessionStatus,
    /// Media configuration
    pub config: MediaConfig,
    /// RTP port allocated for this session
    pub rtp_port: Option<u16>,
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
            status: MediaSessionStatus::Creating,
            config: MediaConfig {
                local_addr: SocketAddr::from(([0, 0, 0, 0], 0)),
                remote_addr: None,
                preferred_codec: None,
                parameters: HashMap::new(),
            },
            rtp_port: None,
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
    SessionFailed { dialog_id: DialogId, error: String },
    /// Remote address updated
    RemoteAddressUpdated {
        dialog_id: DialogId,
        remote_addr: SocketAddr,
    },
    /// Codec changed during session update (e.g., re-INVITE)
    CodecChanged {
        dialog_id: DialogId,
        old_codec: Option<String>,
        new_codec: Option<String>,
        new_payload_type: u8,
        new_clock_rate: u32,
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

/// Advanced processor set for v2 processors per session.
///
/// Each processor (`vad`/`agc`/`aec`) is wrapped in
/// `parking_lot::RwLock` rather than `tokio::sync::RwLock` because
/// the per-frame `process_frame` / `analyze_frame` critical sections
/// are CPU-only DSP work (no `.await`), and on the advanced-media
/// hot path the tokio scheduler dip costs more than the lock itself.
/// `metrics` uses `ConcurrentPerformanceMetrics` for the same
/// reason it does on the controller (lock-free padded atomics).
#[derive(Debug)]
pub struct AdvancedProcessorSet {
    /// Advanced voice activity detector (v2)
    pub vad: Option<Arc<parking_lot::RwLock<AdvancedVoiceActivityDetector>>>,
    /// Advanced automatic gain control (v2)
    pub agc: Option<Arc<parking_lot::RwLock<AdvancedAutomaticGainControl>>>,
    /// Advanced acoustic echo canceller (v2)
    pub aec: Option<Arc<parking_lot::RwLock<AdvancedAcousticEchoCanceller>>>,
    /// Session-specific frame pool (shared reference)
    pub frame_pool: Arc<AudioFramePool>,
    /// SIMD processor for this session
    pub simd_processor: SimdProcessor,
    /// Performance metrics for this session — lock-free padded
    /// atomics (see C20). External readers get a snapshot via
    /// `metrics.snapshot()`.
    pub metrics: Arc<crate::performance::metrics::ConcurrentPerformanceMetrics>,
    /// Configuration used to create these processors
    pub config: AdvancedProcessorConfig,
}

/// RTP session wrapper for MediaSessionController
///
/// This wrapper manages both the RTP session and its associated audio state.
/// It supports two levels of audio control:
/// - `transmission_enabled`: Whether to send any RTP packets at all
/// - `is_muted`: Whether to replace audio with silence (while still sending RTP)
pub struct RtpSessionWrapper {
    /// The actual RTP session
    pub session: Arc<tokio::sync::Mutex<RtpSession>>,
    /// Serializes media configuration generations for this dialog. The guard
    /// is cloned out of the map before awaiting so no DashMap shard is held.
    pub(super) update_lock: Arc<tokio::sync::Mutex<()>>,
    /// Local RTP address
    pub local_addr: SocketAddr,
    /// Remote RTP address (if known)
    pub remote_addr: Option<SocketAddr>,
    /// Session creation time
    pub created_at: Instant,
    /// Audio transmitter for outgoing audio
    pub audio_transmitter: Option<super::audio_generation::AudioTransmitter>,
    /// Whether audio transmission is enabled
    ///
    /// When false, no RTP packets are sent at all. This is used when the
    /// session is completely stopped or paused.
    pub transmission_enabled: bool,
    /// Whether audio is muted (send silence instead of actual audio)
    ///
    /// When true, RTP packets continue to be sent but audio samples are
    /// replaced with silence. This maintains RTP flow for NAT traversal
    /// and prevents remote endpoint timeouts.
    pub is_muted: bool,

    #[cfg(feature = "memory-diagnostics")]
    pub memory_guard: rvoip_infra_common::memory_diagnostics::ObjectGuard,
}
