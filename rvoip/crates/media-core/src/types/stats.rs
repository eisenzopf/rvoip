//! Media statistics types that combine various statistics sources

use super::{DialogId, MediaSessionId};
use rvoip_rtp_core::session::{RtpSessionStats, RtpStreamStats};
use std::time::{Duration, Instant};

/// Comprehensive media statistics for a session
#[derive(Debug, Clone)]
pub struct MediaStatistics {
    /// Session identifiers
    pub session_id: MediaSessionId,
    pub dialog_id: DialogId,
    
    /// RTP/RTCP statistics from rtp-core
    pub rtp_stats: Option<RtpSessionStats>,
    
    /// Per-stream statistics (for multi-stream scenarios)
    pub stream_stats: Vec<RtpStreamStats>,
    
    /// Media processing statistics
    pub media_stats: MediaProcessingStats,
    
    /// Quality metrics
    pub quality_metrics: Option<QualityMetrics>,
    
    /// Session timing
    pub session_start: Instant,
    pub session_duration: Duration,
}

/// Media processing statistics
#[derive(Debug, Clone, Default)]
pub struct MediaProcessingStats {
    /// Packets processed
    pub packets_processed: u64,
    
    /// Frames encoded
    pub frames_encoded: u64,
    
    /// Frames decoded
    pub frames_decoded: u64,
    
    /// Processing errors
    pub processing_errors: u64,
    
    /// Codec changes
    pub codec_changes: u32,
    
    /// Current codec
    pub current_codec: Option<String>,
}

/// Quality metrics with RTCP-derived values
#[derive(Debug, Clone)]
pub struct QualityMetrics {
    /// Packet loss percentage (from RTCP)
    pub packet_loss_percent: f32,
    
    /// Jitter in milliseconds (from RTCP)
    pub jitter_ms: f64,
    
    /// Round-trip time in milliseconds (from RTCP SR/RR)
    pub rtt_ms: Option<f64>,
    
    /// MOS score estimate (1-5)
    pub mos_score: Option<f32>,
    
    /// Network quality indicator (0-100)
    pub network_quality: u8,
}

impl Default for QualityMetrics {
    fn default() -> Self {
        Self {
            packet_loss_percent: 0.0,
            jitter_ms: 0.0,
            rtt_ms: None,
            mos_score: Some(4.5), // Default to excellent
            network_quality: 100,
        }
    }
} 