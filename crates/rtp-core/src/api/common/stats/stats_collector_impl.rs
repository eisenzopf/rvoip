//! Stats collector implementation
//!
//! This file contains the implementation of the MediaStatsCollector trait.

use async_trait::async_trait;
use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime};
use tokio::sync::{Mutex, RwLock};


use crate::api::common::stats::{
    MediaStats, MediaStatsCollector, QualityLevel, StatsError, StreamStats,
};

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


}

#[async_trait]
impl MediaStatsCollector for DefaultMediaStatsCollector {
    async fn get_stats(&self) -> Result<MediaStats, StatsError> {
        let stats = self.stats.read().await;
        Ok(stats.clone())
    }

    async fn get_stream_stats(&self, ssrc: u32) -> Result<StreamStats, StatsError> {
        let stats = self.stats.read().await;
        stats.streams.get(&ssrc).cloned().ok_or_else(|| {
            StatsError::InvalidStreamId(format!("Stream with SSRC {} not found", ssrc))
        })
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
