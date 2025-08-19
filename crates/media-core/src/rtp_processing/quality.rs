//! Media quality monitoring and metrics (moved from rtp-core)

use std::time::{Duration, Instant};

/// Media quality metrics
#[derive(Debug, Clone)]
pub struct QualityMetrics {
    /// Mean Opinion Score (1.0 - 5.0)
    pub mos_score: f32,
    
    /// Packet loss percentage (0.0 - 100.0)
    pub packet_loss: f32,
    
    /// Jitter in milliseconds
    pub jitter_ms: f32,
    
    /// Round-trip time in milliseconds
    pub round_trip_ms: f32,
    
    /// Bitrate in bits per second
    pub bitrate_bps: u32,
    
    /// When these metrics were calculated
    pub timestamp: Instant,
}

impl Default for QualityMetrics {
    fn default() -> Self {
        Self {
            mos_score: 4.0, // Good quality by default
            packet_loss: 0.0,
            jitter_ms: 0.0,
            round_trip_ms: 0.0,
            bitrate_bps: 0,
            timestamp: Instant::now(),
        }
    }
}

/// Configuration for quality monitoring
#[derive(Debug, Clone)]
pub struct QualityMonitorConfig {
    /// How often to calculate quality metrics
    pub update_interval: Duration,
    
    /// Window size for moving averages
    pub window_size: usize,
    
    /// Enable detailed logging
    pub enable_logging: bool,
}

impl Default for QualityMonitorConfig {
    fn default() -> Self {
        Self {
            update_interval: Duration::from_secs(1),
            window_size: 10,
            enable_logging: false,
        }
    }
}

/// Quality monitor for tracking media quality over time
pub struct QualityMonitor {
    config: QualityMonitorConfig,
    current_metrics: QualityMetrics,
    packet_count: u64,
    lost_packets: u64,
    last_update: Instant,
}

impl QualityMonitor {
    /// Create a new quality monitor
    pub fn new(config: QualityMonitorConfig) -> Self {
        Self {
            config,
            current_metrics: QualityMetrics::default(),
            packet_count: 0,
            lost_packets: 0,
            last_update: Instant::now(),
        }
    }
    
    /// Record a received packet
    pub fn record_packet(&mut self, size_bytes: usize, jitter_ms: f32) {
        self.packet_count += 1;
        
        // Update metrics if it's time
        if self.last_update.elapsed() >= self.config.update_interval {
            self.update_metrics();
            self.last_update = Instant::now();
        }
    }
    
    /// Record a lost packet
    pub fn record_lost_packet(&mut self) {
        self.lost_packets += 1;
    }
    
    /// Get current quality metrics
    pub fn get_metrics(&self) -> &QualityMetrics {
        &self.current_metrics
    }
    
    /// Update calculated metrics
    fn update_metrics(&mut self) {
        // Calculate packet loss percentage
        if self.packet_count > 0 {
            self.current_metrics.packet_loss = 
                (self.lost_packets as f32 / (self.packet_count + self.lost_packets) as f32) * 100.0;
        }
        
        // Calculate MOS score based on packet loss and jitter
        self.current_metrics.mos_score = self.calculate_mos_score();
        
        self.current_metrics.timestamp = Instant::now();
    }
    
    /// Calculate MOS score based on current conditions
    fn calculate_mos_score(&self) -> f32 {
        let base_mos = 4.5; // Start with excellent quality
        
        // Reduce based on packet loss
        let loss_penalty = self.current_metrics.packet_loss * 0.05;
        
        // Reduce based on jitter
        let jitter_penalty = (self.current_metrics.jitter_ms / 10.0) * 0.1;
        
        (base_mos - loss_penalty - jitter_penalty).max(1.0).min(5.0)
    }
}