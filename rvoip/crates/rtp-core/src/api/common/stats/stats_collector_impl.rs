//! Stats collector implementation
//!
//! This file contains the implementation of the MediaStatsCollector trait.

use std::collections::HashMap;
use std::net::SocketAddr;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::sync::{Mutex, RwLock};
use async_trait::async_trait;

use tracing::{debug, info, warn};

use crate::api::common::stats::{
    MediaStatsCollector, MediaStats, StreamStats, StatsError,
    QualityLevel, Direction, QualityUtils
};
use crate::api::common::frame::MediaFrameType;

/// Default implementation of MediaStatsCollector
pub struct DefaultMediaStatsCollector {
    /// Global statistics
    stats: RwLock<MediaStats>,
    
    /// Quality change callback
    quality_callback: Mutex<Option<Box<dyn Fn(QualityLevel) + Send + Sync>>>,
    
    /// Bandwidth update callback
    bandwidth_callback: Mutex<Option<Box<dyn Fn(u32) + Send + Sync>>>,
    
    /// Last overall quality level
    last_quality: RwLock<QualityLevel>,
    
    /// Last bandwidth estimate
    last_bandwidth: RwLock<u32>,
    
    /// Start time of the session
    start_time: RwLock<SystemTime>,
}

impl DefaultMediaStatsCollector {
    /// Create a new DefaultMediaStatsCollector
    pub fn new() -> Arc<Self> {
        // Create empty stats
        let stats = MediaStats {
            timestamp: SystemTime::now(),
            session_duration: Duration::from_secs(0),
            streams: HashMap::new(),
            quality: QualityLevel::Unknown,
            upstream_bandwidth_bps: 0,
            downstream_bandwidth_bps: 0,
            available_bandwidth_bps: None,
            network_rtt_ms: None,
        };
        
        Arc::new(Self {
            stats: RwLock::new(stats),
            quality_callback: Mutex::new(None),
            bandwidth_callback: Mutex::new(None),
            last_quality: RwLock::new(QualityLevel::Unknown),
            last_bandwidth: RwLock::new(0),
            start_time: RwLock::new(SystemTime::now()),
        })
    }
    
    /// Update statistics for a stream
    pub async fn update_stream_stats(
        &self,
        ssrc: u32,
        direction: Direction,
        media_type: MediaFrameType,
        remote_addr: SocketAddr,
        packet_count: u64,
        byte_count: u64,
        packets_lost: u64,
        fraction_lost: f32,
        jitter_ms: f32,
        rtt_ms: Option<f32>,
    ) {
        let mut stats = self.stats.write().await;
        
        // Calculate quality for this stream
        let quality = QualityUtils::calculate_quality(fraction_lost, jitter_ms, rtt_ms);
        
        // Calculate MOS if audio stream
        let mos = if media_type == MediaFrameType::Audio {
            // Calculate R-factor first
            let r = 93.2 - (rtt_ms.unwrap_or(0.0) / 2.0) - 2.5 * fraction_lost * 100.0;
            Some(QualityUtils::r_factor_to_mos(r))
        } else {
            None
        };
        
        // Calculate bitrate
        let bitrate_bps = if let Some(stream) = stats.streams.get(&ssrc) {
            // Calculate bitrate based on byte count delta
            if stream.byte_count < byte_count && byte_count > 0 {
                let delta_bytes = byte_count - stream.byte_count;
                let time_delta = stats.timestamp.elapsed().unwrap_or(Duration::from_millis(100));
                let time_secs = time_delta.as_secs_f32();
                if time_secs > 0.0 {
                    (delta_bytes as f32 * 8.0 / time_secs) as u32
                } else {
                    0
                }
            } else {
                0
            }
        } else {
            0
        };
        
        // Create or update stream stats
        let stream_stats = StreamStats {
            direction,
            ssrc,
            media_type,
            packet_count,
            byte_count,
            packets_lost,
            fraction_lost,
            jitter_ms,
            rtt_ms,
            mos,
            remote_addr,
            bitrate_bps,
            discard_rate: 0.0, // Not tracked yet
            quality,
            available_bandwidth_bps: None, // Updated separately
        };
        
        // Insert or update the stream
        stats.streams.insert(ssrc, stream_stats);
        
        // Update global stats
        stats.timestamp = SystemTime::now();
        let start_time = self.start_time.read().await;
        stats.session_duration = stats.timestamp.duration_since(*start_time).unwrap_or(Duration::from_secs(0));
        
        // Update overall quality (worst of any stream)
        let overall_quality = stats.streams.values()
            .map(|s| s.quality)
            .min()
            .unwrap_or(QualityLevel::Unknown);
        
        let current_quality = self.last_quality.read().await;
        if overall_quality != *current_quality {
            // Quality changed, update and notify
            stats.quality = overall_quality;
            
            // Drop the read lock before acquiring the write lock
            drop(current_quality);
            
            let mut last_quality = self.last_quality.write().await;
            *last_quality = overall_quality;
            
            // Notify via callback
            // Use try_lock to avoid blocking in async context
            if let Ok(callback) = self.quality_callback.try_lock() {
                if let Some(cb) = &*callback {
                    cb(overall_quality);
                }
            }
        }
        
        // Update bandwidth totals
        let (mut upstream, mut downstream) = (0, 0);
        for stream in stats.streams.values() {
            match stream.direction {
                Direction::Outbound => upstream += stream.bitrate_bps,
                Direction::Inbound => downstream += stream.bitrate_bps,
            }
        }
        
        stats.upstream_bandwidth_bps = upstream;
        stats.downstream_bandwidth_bps = downstream;
        
        // Get network RTT from any stream with RTT
        stats.network_rtt_ms = stats.streams.values()
            .filter_map(|s| s.rtt_ms)
            .min_by(|a, b| a.partial_cmp(b).unwrap_or(std::cmp::Ordering::Equal));
    }
    
    /// Update available bandwidth estimate
    pub async fn update_bandwidth_estimate(&self, bps: u32) {
        let mut stats = self.stats.write().await;
        stats.available_bandwidth_bps = Some(bps);
        
        // Update all streams
        for stream in stats.streams.values_mut() {
            stream.available_bandwidth_bps = Some(bps);
        }
        
        // Check if bandwidth changed significantly
        let current = *self.last_bandwidth.read().await;
        let change_pct = if current > 0 {
            (bps as f32 - current as f32).abs() / current as f32
        } else {
            1.0 // 100% change if previous was 0
        };
        
        if change_pct > 0.1 { // 10% change threshold
            // Bandwidth changed significantly, notify
            let mut last_bw = self.last_bandwidth.write().await;
            *last_bw = bps;
            
            // Notify via callback
            if let Ok(callback) = self.bandwidth_callback.try_lock() {
                if let Some(cb) = &*callback {
                    cb(bps);
                }
            }
        }
    }
}

#[async_trait]
impl MediaStatsCollector for DefaultMediaStatsCollector {
    async fn get_stats(&self) -> Result<MediaStats, StatsError> {
        let stats = self.stats.read().await;
        Ok(stats.clone())
    }
    
    async fn get_stream_stats(&self, ssrc: u32) -> Result<StreamStats, StatsError> {
        let stats = self.stats.read().await;
        stats.streams.get(&ssrc)
            .cloned()
            .ok_or_else(|| StatsError::InvalidStreamId(format!("Stream with SSRC {} not found", ssrc)))
    }
    
    async fn reset(&self) {
        let current_time = SystemTime::now();
        
        let mut stats = self.stats.write().await;
        stats.streams.clear();
        stats.quality = QualityLevel::Unknown;
        stats.upstream_bandwidth_bps = 0;
        stats.downstream_bandwidth_bps = 0;
        stats.available_bandwidth_bps = None;
        stats.network_rtt_ms = None;
        
        *self.last_quality.write().await = QualityLevel::Unknown;
        *self.last_bandwidth.write().await = 0;
        
        // Update the start time
        *self.start_time.write().await = current_time;
        
        stats.timestamp = current_time;
        stats.session_duration = Duration::from_secs(0);
    }
    
    async fn on_quality_change(&self, callback: Box<dyn Fn(QualityLevel) + Send + Sync>) {
        let mut cb = self.quality_callback.lock().await;
        *cb = Some(callback);
    }
    
    async fn on_bandwidth_update(&self, callback: Box<dyn Fn(u32) + Send + Sync>) {
        let mut cb = self.bandwidth_callback.lock().await;
        *cb = Some(callback);
    }
} 