//! Quality Monitor - Real-time Quality Analysis
//!
//! This module implements real-time quality monitoring and analysis for media sessions,
//! tracking packet loss, jitter, latency, and overall call quality.

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, Instant};
use tokio::sync::RwLock;
use tracing::{debug, warn, trace};

use crate::error::Result;
use crate::types::{MediaSessionId, MediaPacket};
use super::metrics::{QualityMetrics, SessionMetrics, OverallMetrics, QualityThresholds, QualityTrend};

/// Configuration for quality monitoring
#[derive(Debug, Clone)]
pub struct QualityMonitorConfig {
    /// Monitoring interval (how often to calculate metrics)
    pub monitoring_interval: Duration,
    /// Quality thresholds for alerts
    pub thresholds: QualityThresholds,
    /// Number of samples for jitter calculation
    pub jitter_samples: usize,
    /// Enable detailed quality logging
    pub enable_detailed_logging: bool,
    /// MOS calculation parameters
    pub mos_calculation_weight: MosWeights,
}

/// Weights for MOS calculation components
#[derive(Debug, Clone)]
pub struct MosWeights {
    pub packet_loss_weight: f32,
    pub jitter_weight: f32,
    pub latency_weight: f32,
}

impl Default for QualityMonitorConfig {
    fn default() -> Self {
        Self {
            monitoring_interval: Duration::from_secs(5), // Monitor every 5 seconds
            thresholds: QualityThresholds::default(),
            jitter_samples: 20, // Use 20 samples for jitter calculation
            enable_detailed_logging: true,
            mos_calculation_weight: MosWeights {
                packet_loss_weight: 0.4,
                jitter_weight: 0.3,
                latency_weight: 0.3,
            },
        }
    }
}

/// Real-time quality monitor
pub struct QualityMonitor {
    /// Monitor configuration
    config: QualityMonitorConfig,
    /// Session metrics tracking
    session_metrics: Arc<RwLock<HashMap<MediaSessionId, SessionMetrics>>>,
    /// Jitter calculation buffers
    jitter_buffers: Arc<RwLock<HashMap<MediaSessionId, JitterBuffer>>>,
    /// Overall system metrics
    overall_metrics: Arc<RwLock<OverallMetrics>>,
    /// Last monitoring timestamp
    last_monitoring: Arc<RwLock<Instant>>,
}

/// Jitter calculation buffer
#[derive(Debug)]
struct JitterBuffer {
    /// Recent packet intervals
    intervals: Vec<Duration>,
    /// Last packet timestamp
    last_timestamp: Option<Instant>,
    /// Current jitter estimate
    current_jitter: f32,
}

impl QualityMonitor {
    /// Create a new quality monitor
    pub fn new(config: QualityMonitorConfig) -> Self {
        debug!("Creating QualityMonitor with config: {:?}", config);
        
        Self {
            config,
            session_metrics: Arc::new(RwLock::new(HashMap::new())),
            jitter_buffers: Arc::new(RwLock::new(HashMap::new())),
            overall_metrics: Arc::new(RwLock::new(OverallMetrics::default())),
            last_monitoring: Arc::new(RwLock::new(Instant::now())),
        }
    }
    
    /// Analyze incoming media packet for quality metrics
    pub async fn analyze_media_packet(&self, session_id: &MediaSessionId, packet: &MediaPacket) -> Result<()> {
        // Update packet statistics
        {
            let mut metrics = self.session_metrics.write().await;
            let session_metrics = metrics.entry(session_id.clone())
                .or_insert_with(|| SessionMetrics::new(session_id.clone()));
            
            session_metrics.update_packet_stats(packet, true);
        }
        
        // Update jitter calculation
        self.update_jitter_calculation(session_id, packet).await;
        
        // Perform periodic quality analysis
        self.perform_periodic_analysis().await?;
        
        Ok(())
    }
    
    /// Get current session metrics
    pub async fn get_session_metrics(&self, session_id: &MediaSessionId) -> Option<SessionMetrics> {
        let metrics = self.session_metrics.read().await;
        metrics.get(session_id).cloned()
    }
    
    /// Get overall system metrics
    pub async fn get_overall_metrics(&self) -> OverallMetrics {
        self.overall_metrics.read().await.clone()
    }
    
    /// Remove session from monitoring
    pub async fn remove_session(&self, session_id: &MediaSessionId) {
        let mut metrics = self.session_metrics.write().await;
        let mut jitter_buffers = self.jitter_buffers.write().await;
        
        metrics.remove(session_id);
        jitter_buffers.remove(session_id);
        
        debug!("Removed session {} from quality monitoring", session_id);
    }
    
    /// Update jitter calculation for a session
    async fn update_jitter_calculation(&self, session_id: &MediaSessionId, packet: &MediaPacket) {
        let mut buffers = self.jitter_buffers.write().await;
        let buffer = buffers.entry(session_id.clone())
            .or_insert_with(|| JitterBuffer {
                intervals: Vec::with_capacity(self.config.jitter_samples),
                last_timestamp: None,
                current_jitter: 0.0,
            });
        
        let now = packet.received_at;
        
        if let Some(last_time) = buffer.last_timestamp {
            let interval = now.duration_since(last_time);
            
            // Add interval to buffer
            if buffer.intervals.len() >= self.config.jitter_samples {
                buffer.intervals.remove(0);
            }
            buffer.intervals.push(interval);
            
            // Calculate jitter (standard deviation of intervals)
            if buffer.intervals.len() > 1 {
                buffer.current_jitter = calculate_jitter(&buffer.intervals);
            }
        }
        
        buffer.last_timestamp = Some(now);
    }
    
    /// Perform periodic quality analysis
    async fn perform_periodic_analysis(&self) -> Result<()> {
        let now = Instant::now();
        let should_analyze = {
            let last_monitoring = self.last_monitoring.read().await;
            now.duration_since(*last_monitoring) >= self.config.monitoring_interval
        };
        
        if should_analyze {
            self.calculate_quality_metrics().await?;
            self.update_overall_metrics().await;
            
            // Update last monitoring time
            {
                let mut last_monitoring = self.last_monitoring.write().await;
                *last_monitoring = now;
            }
        }
        
        Ok(())
    }
    
    /// Calculate quality metrics for all sessions
    async fn calculate_quality_metrics(&self) -> Result<()> {
        let mut session_metrics = self.session_metrics.write().await;
        let jitter_buffers = self.jitter_buffers.read().await;
        
        for (session_id, metrics) in session_metrics.iter_mut() {
            // Get jitter data
            let jitter_ms = jitter_buffers.get(session_id)
                .map(|buffer| buffer.current_jitter)
                .unwrap_or(0.0);
            
            // Calculate packet loss (simplified)
            let packet_loss = self.calculate_packet_loss(metrics);
            
            // Estimate processing latency (simplified)
            let processing_latency = 10.0; // Placeholder - would be measured from actual processing
            
            // Calculate MOS score
            let mos_score = QualityMetrics::calculate_mos(packet_loss, jitter_ms, processing_latency);
            
            // Create quality metrics
            let quality_metrics = QualityMetrics {
                packet_loss,
                jitter_ms,
                rtt_ms: 0.0, // Would be measured from RTCP reports
                mos_score,
                avg_bitrate: self.calculate_bitrate(metrics),
                snr_db: 20.0, // Would be measured from audio analysis
                processing_latency_ms: processing_latency,
            };
            
            // Update session metrics
            metrics.update(quality_metrics.clone());
            
            // Log quality issues
            if self.config.enable_detailed_logging {
                let trend = metrics.get_trend();
                if metrics.is_quality_poor(&self.config.thresholds) || trend == QualityTrend::Degrading {
                    warn!("Quality issue detected for session {}: MOS={:.2}, trend={:?}", 
                          session_id, quality_metrics.mos_score, trend);
                }
            }
        }
        
        Ok(())
    }
    
    /// Update overall system metrics
    async fn update_overall_metrics(&self) {
        let session_metrics = self.session_metrics.read().await;
        let mut overall = self.overall_metrics.write().await;
        
        overall.active_sessions = session_metrics.len() as u32;
        
        if !session_metrics.is_empty() {
            // Calculate average quality across all sessions
            let total_sessions = session_metrics.len() as f32;
            let mut avg_metrics = QualityMetrics::default();
            
            for metrics in session_metrics.values() {
                avg_metrics.packet_loss += metrics.current.packet_loss / total_sessions;
                avg_metrics.jitter_ms += metrics.current.jitter_ms / total_sessions;
                avg_metrics.mos_score += metrics.current.mos_score / total_sessions;
                avg_metrics.avg_bitrate += metrics.current.avg_bitrate / total_sessions as u32;
            }
            
            overall.avg_quality = avg_metrics;
        }
        
        // Update system resource metrics (simplified)
        overall.cpu_usage = 10.0; // Would be measured from system
        overall.memory_usage = 1024 * 1024 * 50; // 50MB placeholder
        overall.bandwidth_usage = overall.avg_quality.avg_bitrate * overall.active_sessions;
    }
    
    /// Calculate packet loss percentage for a session
    fn calculate_packet_loss(&self, metrics: &SessionMetrics) -> f32 {
        // Simplified packet loss calculation
        // In reality, this would track sequence numbers
        if metrics.packets_received > 100 {
            let expected = metrics.packets_received + 5; // Assume some loss
            ((expected - metrics.packets_received) as f32 / expected as f32) * 100.0
        } else {
            0.0
        }
    }
    
    /// Calculate average bitrate for a session
    fn calculate_bitrate(&self, metrics: &SessionMetrics) -> u32 {
        let duration_secs = metrics.duration.as_secs().max(1);
        ((metrics.bytes_transferred * 8) / duration_secs) as u32
    }
}

/// Calculate jitter from interval measurements
fn calculate_jitter(intervals: &[Duration]) -> f32 {
    if intervals.len() < 2 {
        return 0.0;
    }
    
    // Calculate mean interval
    let mean_interval: f32 = intervals.iter()
        .map(|d| d.as_millis() as f32)
        .sum::<f32>() / intervals.len() as f32;
    
    // Calculate variance
    let variance: f32 = intervals.iter()
        .map(|d| {
            let diff = d.as_millis() as f32 - mean_interval;
            diff * diff
        })
        .sum::<f32>() / (intervals.len() - 1) as f32;
    
    // Return standard deviation (jitter) in milliseconds
    variance.sqrt()
} 