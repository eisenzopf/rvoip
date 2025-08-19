//! Media quality estimation and monitoring (moved from rtp-core)
//!
//! This module handles the estimation of media quality levels based on
//! RTP statistics and provides real-time quality monitoring.

use std::collections::HashMap;
use std::time::{Duration, Instant};
use tracing::{debug, warn};

/// Media quality levels based on ITU-T standards
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaQualityLevel {
    /// Excellent quality (MOS 4.5-5.0)
    Excellent,
    /// Good quality (MOS 3.5-4.5)
    Good,
    /// Fair quality (MOS 2.5-3.5)
    Fair,
    /// Poor quality (MOS 1.5-2.5)
    Poor,
    /// Bad quality (MOS 1.0-1.5)
    Bad,
    /// Quality unknown (insufficient data)
    Unknown,
}

/// Comprehensive media quality metrics
#[derive(Debug, Clone)]
pub struct MediaQualityMetrics {
    /// Overall quality level
    pub quality_level: MediaQualityLevel,
    /// Mean Opinion Score (1.0-5.0)
    pub mos: f32,
    /// Packet loss percentage (0.0-1.0)
    pub packet_loss: f32,
    /// Jitter in milliseconds
    pub jitter_ms: f32,
    /// Round-trip time in milliseconds
    pub rtt_ms: Option<f32>,
    /// Quality score (0-100)
    pub quality_score: f32,
    /// Number of samples used for calculation
    pub sample_count: u32,
    /// Last update timestamp
    pub last_updated: Instant,
}

/// Statistics for a single media stream
#[derive(Debug, Clone)]
pub struct MediaStreamStats {
    /// Stream identifier (SSRC)
    pub ssrc: u32,
    /// Media type (audio/video)
    pub media_type: MediaType,
    /// Stream direction
    pub direction: StreamDirection,
    /// Total packets processed
    pub packet_count: u64,
    /// Total bytes processed
    pub byte_count: u64,
    /// Packets lost
    pub packets_lost: u32,
    /// Fraction lost (0.0-1.0)
    pub fraction_lost: f32,
    /// Jitter in milliseconds
    pub jitter_ms: f32,
    /// Round-trip time
    pub rtt_ms: Option<f32>,
    /// Current bitrate in bps
    pub bitrate_bps: u32,
    /// Discard rate
    pub discard_rate: f32,
    /// Stream quality
    pub quality: MediaQualityLevel,
    /// Remote address
    pub remote_addr: std::net::SocketAddr,
}

/// Media type classification
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaType {
    Audio,
    Video,
    Data,
}

/// Stream direction
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StreamDirection {
    Inbound,
    Outbound,
}

/// Comprehensive media session statistics
#[derive(Debug, Clone)]
pub struct MediaSessionStats {
    /// Session duration
    pub session_duration: Duration,
    /// Overall quality level
    pub quality: MediaQualityLevel,
    /// Downstream bandwidth
    pub downstream_bandwidth_bps: u32,
    /// Upstream bandwidth
    pub upstream_bandwidth_bps: u32,
    /// Per-stream statistics
    pub streams: HashMap<u32, MediaStreamStats>,
    /// Quality metrics history
    pub quality_history: Vec<MediaQualityMetrics>,
}

impl Default for MediaSessionStats {
    fn default() -> Self {
        Self {
            session_duration: Duration::from_secs(0),
            quality: MediaQualityLevel::Unknown,
            downstream_bandwidth_bps: 0,
            upstream_bandwidth_bps: 0,
            streams: HashMap::new(),
            quality_history: Vec::new(),
        }
    }
}

/// Real-time media quality monitor
pub struct MediaQualityMonitor {
    /// Quality metrics per stream
    stream_metrics: HashMap<u32, MediaQualityMetrics>,
    /// Quality history
    quality_history: Vec<MediaQualityMetrics>,
    /// Maximum history size
    max_history: usize,
    /// Minimum samples for quality calculation
    min_samples: u32,
}

impl MediaQualityMonitor {
    /// Create a new quality monitor
    pub fn new() -> Self {
        Self {
            stream_metrics: HashMap::new(),
            quality_history: Vec::new(),
            max_history: 1000, // Keep last 1000 quality measurements
            min_samples: 10,   // Need at least 10 packets for quality calc
        }
    }
    
    /// Update quality metrics for a stream
    pub fn update_stream_quality(&mut self, ssrc: u32, stats: &MediaStreamStats) -> MediaQualityMetrics {
        let quality_metrics = self.calculate_quality_metrics(stats);
        self.stream_metrics.insert(ssrc, quality_metrics.clone());
        
        // Add to history
        self.quality_history.push(quality_metrics.clone());
        
        // Trim history if too large
        if self.quality_history.len() > self.max_history {
            self.quality_history.remove(0);
        }
        
        quality_metrics
    }
    
    /// Calculate quality metrics from stream statistics
    fn calculate_quality_metrics(&self, stats: &MediaStreamStats) -> MediaQualityMetrics {
        // Not enough data for quality estimation
        if stats.packet_count < self.min_samples as u64 {
            return MediaQualityMetrics {
                quality_level: MediaQualityLevel::Unknown,
                mos: 0.0,
                packet_loss: 0.0,
                jitter_ms: 0.0,
                rtt_ms: None,
                quality_score: 0.0,
                sample_count: stats.packet_count as u32,
                last_updated: Instant::now(),
            };
        }
        
        let packet_loss = stats.fraction_lost;
        let jitter_ms = stats.jitter_ms;
        let rtt_ms = stats.rtt_ms.unwrap_or(0.0);
        
        // Calculate component scores (0-100, higher is better)
        let packet_loss_score = calculate_packet_loss_score(packet_loss);
        let jitter_score = calculate_jitter_score(jitter_ms);
        let rtt_score = calculate_rtt_score(rtt_ms);
        
        // Calculate overall quality score with weights:
        // - Packet loss is most important (weight 50%)
        // - Jitter is second (weight 30%)
        // - RTT is third (weight 20%)
        let quality_score = packet_loss_score * 0.5 + jitter_score * 0.3 + rtt_score * 0.2;
        
        // Map score to quality level and MOS
        let (quality_level, mos) = map_score_to_quality(quality_score);
        
        MediaQualityMetrics {
            quality_level,
            mos,
            packet_loss,
            jitter_ms,
            rtt_ms: stats.rtt_ms,
            quality_score,
            sample_count: stats.packet_count as u32,
            last_updated: Instant::now(),
        }
    }
    
    /// Get current quality metrics for a stream
    pub fn get_stream_quality(&self, ssrc: u32) -> Option<&MediaQualityMetrics> {
        self.stream_metrics.get(&ssrc)
    }
    
    /// Get overall session quality based on all streams
    pub fn get_session_quality(&self) -> MediaQualityLevel {
        if self.stream_metrics.is_empty() {
            return MediaQualityLevel::Unknown;
        }
        
        // Calculate average quality level
        let mut quality_sum = 0.0;
        let mut count = 0;
        
        for metrics in self.stream_metrics.values() {
            if metrics.quality_level != MediaQualityLevel::Unknown {
                quality_sum += metrics.quality_score;
                count += 1;
            }
        }
        
        if count == 0 {
            return MediaQualityLevel::Unknown;
        }
        
        let avg_score = quality_sum / count as f32;
        map_score_to_quality(avg_score).0
    }
    
    /// Get quality history
    pub fn get_quality_history(&self) -> &[MediaQualityMetrics] {
        &self.quality_history
    }
    
    /// Clear all metrics
    pub fn clear(&mut self) {
        self.stream_metrics.clear();
        self.quality_history.clear();
    }
}

impl Default for MediaQualityMonitor {
    fn default() -> Self {
        Self::new()
    }
}

/// Calculate packet loss score (0-100, higher is better)
fn calculate_packet_loss_score(packet_loss: f32) -> f32 {
    if packet_loss <= 0.001 { // Less than 0.1% loss
        100.0
    } else if packet_loss <= 0.005 { // Less than 0.5% loss
        95.0
    } else if packet_loss <= 0.01 { // Less than 1% loss
        90.0
    } else if packet_loss <= 0.03 { // Less than 3% loss
        70.0
    } else if packet_loss <= 0.05 { // Less than 5% loss
        50.0
    } else if packet_loss <= 0.10 { // Less than 10% loss
        30.0
    } else if packet_loss <= 0.20 { // Less than 20% loss
        10.0
    } else { // 20% or more loss
        0.0
    }
}

/// Calculate jitter score (0-100, higher is better)
fn calculate_jitter_score(jitter_ms: f32) -> f32 {
    if jitter_ms <= 1.0 {
        100.0
    } else if jitter_ms <= 5.0 {
        90.0
    } else if jitter_ms <= 10.0 {
        80.0
    } else if jitter_ms <= 20.0 {
        60.0
    } else if jitter_ms <= 30.0 {
        40.0
    } else if jitter_ms <= 50.0 {
        20.0
    } else {
        0.0
    }
}

/// Calculate RTT score (0-100, higher is better)
fn calculate_rtt_score(rtt_ms: f32) -> f32 {
    if rtt_ms == 0.0 { // Unknown RTT
        50.0 // Neutral score
    } else if rtt_ms <= 20.0 {
        100.0
    } else if rtt_ms <= 50.0 {
        90.0
    } else if rtt_ms <= 100.0 {
        80.0
    } else if rtt_ms <= 150.0 {
        60.0
    } else if rtt_ms <= 200.0 {
        40.0
    } else if rtt_ms <= 300.0 {
        20.0
    } else {
        0.0
    }
}

/// Map quality score to quality level and MOS
fn map_score_to_quality(score: f32) -> (MediaQualityLevel, f32) {
    match score {
        x if x >= 90.0 => (MediaQualityLevel::Excellent, 4.5 + (x - 90.0) / 20.0), // MOS 4.5-5.0
        x if x >= 75.0 => (MediaQualityLevel::Good, 3.5 + (x - 75.0) / 15.0),        // MOS 3.5-4.5
        x if x >= 60.0 => (MediaQualityLevel::Fair, 2.5 + (x - 60.0) / 15.0),        // MOS 2.5-3.5
        x if x >= 40.0 => (MediaQualityLevel::Poor, 1.5 + (x - 40.0) / 20.0),        // MOS 1.5-2.5
        x => (MediaQualityLevel::Bad, 1.0 + x / 40.0),                               // MOS 1.0-1.5
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::net::{IpAddr, Ipv4Addr, SocketAddr};
    
    fn create_test_stream_stats(packet_loss: f32, jitter_ms: f32, rtt_ms: Option<f32>) -> MediaStreamStats {
        MediaStreamStats {
            ssrc: 12345,
            media_type: MediaType::Audio,
            direction: StreamDirection::Inbound,
            packet_count: 1000,
            byte_count: 160000,
            packets_lost: (packet_loss * 1000.0) as u32,
            fraction_lost: packet_loss,
            jitter_ms,
            rtt_ms,
            bitrate_bps: 64000,
            discard_rate: 0.0,
            quality: MediaQualityLevel::Unknown,
            remote_addr: SocketAddr::new(IpAddr::V4(Ipv4Addr::new(192, 168, 1, 100)), 5004),
        }
    }
    
    #[test]
    fn test_excellent_quality() {
        let mut monitor = MediaQualityMonitor::new();
        let stats = create_test_stream_stats(0.0, 1.0, Some(20.0));
        
        let metrics = monitor.update_stream_quality(12345, &stats);
        
        assert_eq!(metrics.quality_level, MediaQualityLevel::Excellent);
        assert!(metrics.mos >= 4.5);
        assert!(metrics.quality_score >= 90.0);
    }
    
    #[test]
    fn test_poor_quality() {
        let mut monitor = MediaQualityMonitor::new();
        let stats = create_test_stream_stats(0.1, 50.0, Some(300.0)); // 10% loss, high jitter, high RTT
        
        let metrics = monitor.update_stream_quality(12345, &stats);
        
        assert_eq!(metrics.quality_level, MediaQualityLevel::Poor);
        assert!(metrics.mos < 2.5);
        assert!(metrics.quality_score < 60.0);
    }
    
    #[test]
    fn test_insufficient_data() {
        let mut monitor = MediaQualityMonitor::new();
        let mut stats = create_test_stream_stats(0.0, 1.0, Some(20.0));
        stats.packet_count = 5; // Below minimum threshold
        
        let metrics = monitor.update_stream_quality(12345, &stats);
        
        assert_eq!(metrics.quality_level, MediaQualityLevel::Unknown);
        assert_eq!(metrics.mos, 0.0);
    }
    
    #[test]
    fn test_session_quality_aggregation() {
        let mut monitor = MediaQualityMonitor::new();
        
        // Add excellent stream
        let stats1 = create_test_stream_stats(0.0, 1.0, Some(20.0));
        monitor.update_stream_quality(12345, &stats1);
        
        // Add poor stream  
        let stats2 = create_test_stream_stats(0.1, 50.0, Some(300.0));
        monitor.update_stream_quality(67890, &stats2);
        
        let session_quality = monitor.get_session_quality();
        
        // Should be somewhere between excellent and poor
        assert!(matches!(session_quality, MediaQualityLevel::Fair | MediaQualityLevel::Good));
    }
}