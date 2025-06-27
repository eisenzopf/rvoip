//! Quality Metrics Collection and Analysis
//!
//! This module defines quality metrics structures and calculation utilities
//! for monitoring media session performance.

use std::collections::VecDeque;
use std::time::{Duration, Instant};
use crate::types::{MediaSessionId, MediaPacket};

/// Comprehensive quality metrics for a media session
#[derive(Debug, Clone, Default)]
pub struct QualityMetrics {
    /// Packet loss percentage (0.0-100.0)
    pub packet_loss: f32,
    /// Average jitter in milliseconds
    pub jitter_ms: f32,
    /// Round-trip time in milliseconds
    pub rtt_ms: f32,
    /// Audio quality score (1.0-5.0, where 5.0 is excellent)
    pub mos_score: f32,
    /// Average bitrate in bps
    pub avg_bitrate: u32,
    /// Signal-to-noise ratio
    pub snr_db: f32,
    /// Processing latency in milliseconds
    pub processing_latency_ms: f32,
}

/// Session-specific metrics with temporal data
#[derive(Debug, Clone)]
pub struct SessionMetrics {
    /// Session identifier
    pub session_id: MediaSessionId,
    /// Current quality metrics
    pub current: QualityMetrics,
    /// Quality history (last N measurements)
    pub history: VecDeque<QualityMetrics>,
    /// Session duration
    pub duration: Duration,
    /// Total packets received
    pub packets_received: u64,
    /// Total packets sent
    pub packets_sent: u64,
    /// Total bytes transferred
    pub bytes_transferred: u64,
    /// Last update timestamp
    pub last_updated: Instant,
}

/// Overall system metrics across all sessions
#[derive(Debug, Clone, Default)]
pub struct OverallMetrics {
    /// Number of active sessions
    pub active_sessions: u32,
    /// Average quality across all sessions
    pub avg_quality: QualityMetrics,
    /// System resource utilization
    pub cpu_usage: f32,
    /// Memory usage in bytes
    pub memory_usage: u64,
    /// Network bandwidth utilization
    pub bandwidth_usage: u32,
}

/// Quality threshold configuration
#[derive(Debug, Clone)]
pub struct QualityThresholds {
    /// Critical packet loss threshold (%)
    pub critical_packet_loss: f32,
    /// High jitter threshold (ms)
    pub high_jitter: f32,
    /// Poor MOS score threshold
    pub poor_mos: f32,
    /// High latency threshold (ms)
    pub high_latency: f32,
}

impl Default for QualityThresholds {
    fn default() -> Self {
        Self {
            critical_packet_loss: 5.0,  // 5% packet loss is critical
            high_jitter: 30.0,           // 30ms jitter is high
            poor_mos: 2.5,               // MOS below 2.5 is poor
            high_latency: 150.0,         // 150ms latency is high
        }
    }
}

impl SessionMetrics {
    /// Create new session metrics
    pub fn new(session_id: MediaSessionId) -> Self {
        Self {
            session_id,
            current: QualityMetrics::default(),
            history: VecDeque::with_capacity(100), // Keep last 100 measurements
            duration: Duration::ZERO,
            packets_received: 0,
            packets_sent: 0,
            bytes_transferred: 0,
            last_updated: Instant::now(),
        }
    }
    
    /// Update metrics with new measurement
    pub fn update(&mut self, metrics: QualityMetrics) {
        // Store previous metrics in history
        if self.history.len() >= 100 {
            self.history.pop_front();
        }
        self.history.push_back(self.current.clone());
        
        // Update current metrics
        self.current = metrics;
        self.last_updated = Instant::now();
    }
    
    /// Get quality trend (improving, stable, degrading)
    pub fn get_trend(&self) -> QualityTrend {
        if self.history.len() < 3 {
            return QualityTrend::Stable;
        }
        
        let recent: Vec<_> = self.history.iter().rev().take(3).collect();
        let latest_mos = self.current.mos_score;
        let avg_recent_mos = recent.iter().map(|m| m.mos_score).sum::<f32>() / recent.len() as f32;
        
        let trend_threshold = 0.2; // MOS difference threshold
        
        if latest_mos > avg_recent_mos + trend_threshold {
            QualityTrend::Improving
        } else if latest_mos < avg_recent_mos - trend_threshold {
            QualityTrend::Degrading
        } else {
            QualityTrend::Stable
        }
    }
    
    /// Check if quality is below thresholds
    pub fn is_quality_poor(&self, thresholds: &QualityThresholds) -> bool {
        self.current.packet_loss > thresholds.critical_packet_loss ||
        self.current.jitter_ms > thresholds.high_jitter ||
        self.current.mos_score < thresholds.poor_mos ||
        self.current.processing_latency_ms > thresholds.high_latency
    }
    
    /// Update packet statistics
    pub fn update_packet_stats(&mut self, packet: &MediaPacket, is_received: bool) {
        if is_received {
            self.packets_received += 1;
        } else {
            self.packets_sent += 1;
        }
        self.bytes_transferred += packet.payload.len() as u64;
    }
}

impl QualityMetrics {
    /// Calculate MOS (Mean Opinion Score) from technical metrics
    pub fn calculate_mos(packet_loss: f32, jitter_ms: f32, latency_ms: f32) -> f32 {
        // Simplified MOS calculation based on ITU-T P.800
        let mut mos = 4.5; // Start with good quality
        
        // Reduce score based on packet loss
        mos -= packet_loss * 0.1; // 10% packet loss = -1.0 MOS
        
        // Reduce score based on jitter
        if jitter_ms > 20.0 {
            mos -= (jitter_ms - 20.0) * 0.02; // High jitter penalty
        }
        
        // Reduce score based on latency
        if latency_ms > 150.0 {
            mos -= (latency_ms - 150.0) * 0.01; // High latency penalty
        }
        
        // Clamp to valid MOS range (1.0-5.0)
        mos.max(1.0).min(5.0)
    }
    
    /// Get quality grade from MOS score
    pub fn get_quality_grade(&self) -> QualityGrade {
        match self.mos_score {
            mos if mos >= 4.0 => QualityGrade::Excellent,
            mos if mos >= 3.5 => QualityGrade::Good,
            mos if mos >= 2.5 => QualityGrade::Fair,
            mos if mos >= 1.5 => QualityGrade::Poor,
            _ => QualityGrade::Bad,
        }
    }
}

/// Quality trend indicators
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QualityTrend {
    Improving,
    Stable,
    Degrading,
}

/// Quality grades based on MOS scores
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum QualityGrade {
    Excellent, // 4.0-5.0
    Good,      // 3.5-4.0
    Fair,      // 2.5-3.5
    Poor,      // 1.5-2.5
    Bad,       // 1.0-1.5
} 