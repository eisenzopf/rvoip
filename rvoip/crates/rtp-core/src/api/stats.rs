//! Statistics API
//!
//! This module provides a simplified interface for media transport statistics
//! and quality monitoring.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use thiserror::Error;
use async_trait::async_trait;

// Implementation module
mod stats_collector_impl;

/// Error types for statistics operations
#[derive(Error, Debug)]
pub enum StatsError {
    /// No statistics available
    #[error("No statistics available")]
    NoStatsAvailable,
    
    /// Invalid stream identifier
    #[error("Invalid stream identifier: {0}")]
    InvalidStreamId(String),
    
    /// Other statistics error
    #[error("Statistics error: {0}")]
    Other(String),
}

/// Quality level of the media transport
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QualityLevel {
    /// Excellent quality, no noticeable issues
    Excellent,
    
    /// Good quality, minor issues that do not affect user experience
    Good,
    
    /// Fair quality, some noticeable issues but usable
    Fair,
    
    /// Poor quality, significant issues affecting user experience
    Poor,
    
    /// Bad quality, unusable
    Bad,
    
    /// Unknown quality level
    Unknown,
}

/// Direction of media flow
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Direction {
    /// Inbound (receiving)
    Inbound,
    
    /// Outbound (sending)
    Outbound,
}

/// Media statistics for a stream
#[derive(Debug, Clone)]
pub struct StreamStats {
    /// Direction of this stream
    pub direction: Direction,
    
    /// Synchronization source identifier
    pub ssrc: u32,
    
    /// Media type (audio, video, data)
    pub media_type: crate::api::transport::MediaFrameType,
    
    /// Total packets sent or received
    pub packet_count: u64,
    
    /// Total bytes sent or received
    pub byte_count: u64,
    
    /// Number of packets lost
    pub packets_lost: u64,
    
    /// Fraction of packets lost (0.0 - 1.0)
    pub fraction_lost: f32,
    
    /// Jitter in milliseconds
    pub jitter_ms: f32,
    
    /// Round-trip time in milliseconds (if available)
    pub rtt_ms: Option<f32>,
    
    /// Mean Opinion Score (1.0 - 5.0, if available)
    pub mos: Option<f32>,
    
    /// Remote address for this stream
    pub remote_addr: SocketAddr,
    
    /// Average bitrate in bits per second
    pub bitrate_bps: u32,
    
    /// Packet discard rate (0.0 - 1.0)
    pub discard_rate: f32,
    
    /// Current quality level
    pub quality: QualityLevel,
    
    /// Estimated available bandwidth in bits per second
    pub available_bandwidth_bps: Option<u32>,
}

/// Overall media transport statistics
#[derive(Debug, Clone)]
pub struct MediaStats {
    /// Timestamp when these statistics were collected
    pub timestamp: SystemTime,
    
    /// Duration since the start of the session
    pub session_duration: Duration,
    
    /// Map of stream statistics by SSRC
    pub streams: HashMap<u32, StreamStats>,
    
    /// Overall quality level
    pub quality: QualityLevel,
    
    /// Aggregate upstream bandwidth in bits per second
    pub upstream_bandwidth_bps: u32,
    
    /// Aggregate downstream bandwidth in bits per second
    pub downstream_bandwidth_bps: u32,
    
    /// Estimated available bandwidth in bits per second
    pub available_bandwidth_bps: Option<u32>,
    
    /// Current network round-trip time in milliseconds
    pub network_rtt_ms: Option<f32>,
}

/// Media statistics collector interface
#[async_trait]
pub trait MediaStatsCollector: Send + Sync {
    /// Get current media statistics
    async fn get_stats(&self) -> Result<MediaStats, StatsError>;
    
    /// Get statistics for a specific stream by SSRC
    async fn get_stream_stats(&self, ssrc: u32) -> Result<StreamStats, StatsError>;
    
    /// Reset statistics
    async fn reset(&self);
    
    /// Register a callback for quality changes
    async fn on_quality_change(&self, callback: Box<dyn Fn(QualityLevel) + Send + Sync>);
    
    /// Register a callback for bandwidth estimation changes
    async fn on_bandwidth_update(&self, callback: Box<dyn Fn(u32) + Send + Sync>);
}

/// Factory for creating MediaStatsCollector instances
pub struct StatsFactory;

impl StatsFactory {
    /// Create a new MediaStatsCollector
    pub fn create_collector() -> Arc<dyn MediaStatsCollector> {
        stats_collector_impl::DefaultMediaStatsCollector::new()
    }
}

/// Quality utilities
pub struct QualityUtils;

impl QualityUtils {
    /// Convert MOS score to quality level
    pub fn mos_to_quality(mos: f32) -> QualityLevel {
        match mos {
            x if x >= 4.3 => QualityLevel::Excellent,
            x if x >= 3.6 => QualityLevel::Good,
            x if x >= 2.6 => QualityLevel::Fair,
            x if x >= 1.6 => QualityLevel::Poor,
            x if x >= 1.0 => QualityLevel::Bad,
            _ => QualityLevel::Unknown,
        }
    }
    
    /// Calculate MOS from R-factor
    pub fn r_factor_to_mos(r: f32) -> f32 {
        if r < 0.0 {
            return 1.0;
        }
        
        let mos = if r < 100.0 {
            1.0 + 0.035 * r + 0.000007 * r * (r - 60.0) * (100.0 - r)
        } else {
            4.5
        };
        
        mos.max(1.0).min(5.0)
    }
    
    /// Calculate quality level from network metrics
    pub fn calculate_quality(
        packet_loss: f32,
        jitter_ms: f32,
        rtt_ms: Option<f32>,
    ) -> QualityLevel {
        // This is a simplified quality calculation
        // A more sophisticated algorithm would be implemented in the real code
        
        // Start with excellent and degrade based on metrics
        let mut quality = QualityLevel::Excellent;
        
        // Degrade based on packet loss
        quality = match packet_loss {
            x if x < 0.01 => quality,
            x if x < 0.03 => quality.min(QualityLevel::Good),
            x if x < 0.08 => quality.min(QualityLevel::Fair),
            x if x < 0.15 => quality.min(QualityLevel::Poor),
            _ => QualityLevel::Bad,
        };
        
        // Degrade based on jitter
        quality = match jitter_ms {
            x if x < 10.0 => quality,
            x if x < 30.0 => quality.min(QualityLevel::Good),
            x if x < 50.0 => quality.min(QualityLevel::Fair),
            x if x < 100.0 => quality.min(QualityLevel::Poor),
            _ => QualityLevel::Bad,
        };
        
        // Degrade based on RTT if available
        if let Some(rtt) = rtt_ms {
            quality = match rtt {
                x if x < 100.0 => quality,
                x if x < 200.0 => quality.min(QualityLevel::Good),
                x if x < 400.0 => quality.min(QualityLevel::Fair),
                x if x < 1000.0 => quality.min(QualityLevel::Poor),
                _ => QualityLevel::Bad,
            };
        }
        
        quality
    }
}

impl Ord for QualityLevel {
    fn cmp(&self, other: &Self) -> std::cmp::Ordering {
        // Define ordering for quality levels, with Excellent being highest
        let self_val = match self {
            QualityLevel::Excellent => 5,
            QualityLevel::Good => 4,
            QualityLevel::Fair => 3,
            QualityLevel::Poor => 2,
            QualityLevel::Bad => 1,
            QualityLevel::Unknown => 0,
        };
        
        let other_val = match other {
            QualityLevel::Excellent => 5,
            QualityLevel::Good => 4,
            QualityLevel::Fair => 3,
            QualityLevel::Poor => 2,
            QualityLevel::Bad => 1,
            QualityLevel::Unknown => 0,
        };
        
        self_val.cmp(&other_val)
    }
}

impl PartialOrd for QualityLevel {
    fn partial_cmp(&self, other: &Self) -> Option<std::cmp::Ordering> {
        Some(self.cmp(other))
    }
} 