//! Media statistics types for session-core
//!
//! This module defines comprehensive statistics types that aggregate
//! various metrics from different sources for easy consumption.

use std::time::Duration;
use crate::api::types::{SessionId, CallState};
use serde::{Serialize, Deserialize};

/// Comprehensive call statistics aggregating all metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct CallStatistics {
    /// The session ID these statistics belong to
    pub session_id: SessionId,
    
    /// Duration of the call (if started)
    pub duration: Option<Duration>,
    
    /// Current state of the call
    pub state: CallState,
    
    /// Media-specific statistics
    pub media: MediaStatistics,
    
    /// RTP/RTCP statistics
    pub rtp: RtpSessionStats,
    
    /// Quality metrics
    pub quality: QualityMetrics,
}

/// Media session statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct MediaStatistics {
    /// Local media address
    pub local_addr: Option<String>,
    
    /// Remote media address
    pub remote_addr: Option<String>,
    
    /// Negotiated codec
    pub codec: Option<String>,
    
    /// Whether media is currently flowing
    pub media_flowing: bool,
}

/// RTP session statistics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RtpSessionStats {
    /// Number of packets sent
    pub packets_sent: u64,
    
    /// Number of packets received
    pub packets_received: u64,
    
    /// Number of bytes sent
    pub bytes_sent: u64,
    
    /// Number of bytes received
    pub bytes_received: u64,
    
    /// Packet loss count
    pub packets_lost: u64,
    
    /// Out of order packets
    pub packets_out_of_order: u64,
    
    /// Jitter buffer depth in milliseconds
    pub jitter_buffer_ms: f32,
    
    /// Current bitrate in kbps
    pub current_bitrate_kbps: u32,
}

/// Call quality metrics
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct QualityMetrics {
    /// Mean Opinion Score (1.0-5.0)
    pub mos_score: f32,
    
    /// Packet loss rate as percentage
    pub packet_loss_rate: f32,
    
    /// Average jitter in milliseconds
    pub jitter_ms: f32,
    
    /// Round trip time in milliseconds
    pub round_trip_ms: f32,
    
    /// Network effectiveness ratio (0.0-1.0)
    pub network_effectiveness: f32,
    
    /// Whether quality is acceptable
    pub is_acceptable: bool,
}

/// Quality monitoring thresholds for automatic alerts
#[derive(Debug, Clone)]
pub struct QualityThresholds {
    /// Minimum acceptable MOS score (default: 3.0)
    pub min_mos: f32,
    
    /// Maximum acceptable packet loss percentage (default: 5.0)
    pub max_packet_loss: f32,
    
    /// Maximum acceptable jitter in milliseconds (default: 50.0)
    pub max_jitter_ms: f32,
    
    /// Interval between quality checks (default: 5 seconds)
    pub check_interval: Duration,
}

impl Default for MediaStatistics {
    fn default() -> Self {
        Self {
            local_addr: None,
            remote_addr: None,
            codec: None,
            media_flowing: false,
        }
    }
}

impl Default for RtpSessionStats {
    fn default() -> Self {
        Self {
            packets_sent: 0,
            packets_received: 0,
            bytes_sent: 0,
            bytes_received: 0,
            packets_lost: 0,
            packets_out_of_order: 0,
            jitter_buffer_ms: 0.0,
            current_bitrate_kbps: 0,
        }
    }
}

impl Default for QualityMetrics {
    fn default() -> Self {
        Self {
            mos_score: 5.0,
            packet_loss_rate: 0.0,
            jitter_ms: 0.0,
            round_trip_ms: 0.0,
            network_effectiveness: 1.0,
            is_acceptable: true,
        }
    }
}

impl Default for QualityThresholds {
    fn default() -> Self {
        Self {
            min_mos: 3.0,
            max_packet_loss: 5.0,
            max_jitter_ms: 50.0,
            check_interval: Duration::from_secs(5),
        }
    }
}

impl QualityMetrics {
    /// Create a new QualityMetrics with calculated MOS score
    pub fn calculate(packet_loss: f32, jitter_ms: f32, rtt_ms: f32) -> Self {
        // Simple MOS calculation based on E-model
        // This is a simplified version - real implementations use more complex formulas
        let mut mos = 5.0;
        
        // Deduct for packet loss (up to 2.5 points)
        mos -= (packet_loss / 10.0).min(2.5);
        
        // Deduct for jitter (up to 1.0 points)
        mos -= (jitter_ms / 100.0).min(1.0);
        
        // Deduct for latency (up to 0.5 points)
        mos -= (rtt_ms / 500.0).min(0.5);
        
        // Ensure MOS is between 1.0 and 5.0
        mos = mos.max(1.0).min(5.0);
        
        // Calculate network effectiveness
        let effectiveness = if packet_loss > 0.0 {
            1.0 - (packet_loss / 100.0)
        } else {
            1.0
        };
        
        Self {
            mos_score: mos,
            packet_loss_rate: packet_loss,
            jitter_ms,
            round_trip_ms: rtt_ms,
            network_effectiveness: effectiveness,
            is_acceptable: mos >= 3.0,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_quality_metrics_calculation() {
        // Perfect quality
        let perfect = QualityMetrics::calculate(0.0, 0.0, 0.0);
        assert_eq!(perfect.mos_score, 5.0);
        assert!(perfect.is_acceptable);
        
        // Poor quality
        let poor = QualityMetrics::calculate(20.0, 150.0, 400.0);
        assert!(poor.mos_score < 3.0);
        assert!(!poor.is_acceptable);
        
        // Acceptable quality
        let acceptable = QualityMetrics::calculate(2.0, 30.0, 100.0);
        assert!(acceptable.mos_score >= 3.0);
        assert!(acceptable.is_acceptable);
    }
} 