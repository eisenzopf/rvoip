//! Feedback Generation Algorithms
//!
//! This module implements intelligent feedback generators that analyze network conditions,
//! packet loss patterns, and quality metrics to make decisions about when to generate
//! RTCP feedback packets for optimal media quality adaptation.

use crate::{Result, RtpSsrc};
use crate::feedback::{
    FeedbackGenerator, FeedbackContext, FeedbackConfig, FeedbackDecision, 
    FeedbackPriority, QualityDegradation, CongestionState
};
use crate::api::common::stats::StreamStats;
use std::time::{Instant, Duration};
use std::collections::VecDeque;

/// Loss-based feedback generator
/// Generates PLI/FIR packets based on packet loss patterns
pub struct LossFeedbackGenerator {
    /// Recent loss events for trend analysis
    loss_history: VecDeque<LossEvent>,
    
    /// Last PLI generation time
    last_pli: Option<Instant>,
    
    /// Last FIR generation time
    last_fir: Option<Instant>,
    
    /// Current FIR sequence number
    fir_sequence: u8,
    
    /// Consecutive packet loss counter
    consecutive_loss: u16,
    
    /// Total packets processed
    total_packets: u32,
    
    /// Total packets lost
    total_lost: u32,
}

#[derive(Debug, Clone)]
struct LossEvent {
    timestamp: Instant,
    loss_rate: f32,
    consecutive_count: u16,
}

impl LossFeedbackGenerator {
    /// Create a new loss-based feedback generator
    pub fn new() -> Self {
        Self {
            loss_history: VecDeque::new(),
            last_pli: None,
            last_fir: None,
            fir_sequence: 0,
            consecutive_loss: 0,
            total_packets: 0,
            total_lost: 0,
        }
    }
    
    /// Calculate current loss rate
    fn current_loss_rate(&self) -> f32 {
        if self.total_packets == 0 {
            return 0.0;
        }
        self.total_lost as f32 / self.total_packets as f32
    }
    
    /// Check if we should generate PLI based on loss patterns
    fn should_generate_pli(&self, context: &FeedbackContext, config: &FeedbackConfig) -> bool {
        if !config.enable_pli {
            return false;
        }
        
        // Check minimum interval
        if let Some(last) = self.last_pli {
            if last.elapsed().as_millis() < config.pli_interval_ms as u128 {
                return false;
            }
        }
        
        let loss_rate = self.current_loss_rate();
        
        // Generate PLI if:
        // 1. Loss rate exceeds 1%
        // 2. Consecutive losses exceed 3 packets
        // 3. Congestion state is moderate or higher
        loss_rate > 0.01 || 
        self.consecutive_loss > 3 ||
        matches!(context.congestion_state, CongestionState::Moderate | CongestionState::Severe | CongestionState::Critical)
    }
    
    /// Check if we should generate FIR based on severe loss
    fn should_generate_fir(&self, context: &FeedbackContext, config: &FeedbackConfig) -> bool {
        if !config.enable_fir {
            return false;
        }
        
        // Check minimum interval
        if let Some(last) = self.last_fir {
            if last.elapsed().as_millis() < config.fir_interval_ms as u128 {
                return false;
            }
        }
        
        let loss_rate = self.current_loss_rate();
        
        // Generate FIR for severe conditions:
        // 1. Loss rate exceeds 5%
        // 2. Consecutive losses exceed 10 packets
        // 3. Critical congestion state
        loss_rate > 0.05 || 
        self.consecutive_loss > 10 ||
        matches!(context.congestion_state, CongestionState::Critical)
    }
    
    /// Determine feedback priority based on loss severity
    fn calculate_priority(&self, loss_rate: f32) -> FeedbackPriority {
        match loss_rate {
            rate if rate > 0.10 => FeedbackPriority::Critical,  // >10% loss
            rate if rate > 0.05 => FeedbackPriority::High,      // >5% loss
            rate if rate > 0.02 => FeedbackPriority::Normal,    // >2% loss
            _ => FeedbackPriority::Low,
        }
    }
}

impl FeedbackGenerator for LossFeedbackGenerator {
    fn generate_feedback(&self, context: &FeedbackContext, config: &FeedbackConfig) -> Result<FeedbackDecision> {
        let loss_rate = self.current_loss_rate();
        
        if self.should_generate_fir(context, config) {
            return Ok(FeedbackDecision::Fir {
                priority: FeedbackPriority::Critical,
                sequence_number: self.fir_sequence,
            });
        }
        
        if self.should_generate_pli(context, config) {
            let priority = self.calculate_priority(loss_rate);
            let reason = QualityDegradation::PacketLoss {
                rate: (loss_rate * 100.0) as u8,
                consecutive: self.consecutive_loss,
            };
            
            return Ok(FeedbackDecision::Pli {
                priority,
                reason,
            });
        }
        
        Ok(FeedbackDecision::None)
    }
    
    fn update_statistics(&mut self, stats: &StreamStats) {
        // Update packet counters
        self.total_packets = stats.packet_count as u32;
        self.total_lost = stats.packets_lost as u32;
        
        // Track consecutive losses
        if stats.packets_lost > 0 {
            self.consecutive_loss += 1;
        } else {
            self.consecutive_loss = 0;
        }
        
        // Record loss event
        let loss_rate = self.current_loss_rate();
        if loss_rate > 0.0 {
            self.loss_history.push_back(LossEvent {
                timestamp: Instant::now(),
                loss_rate,
                consecutive_count: self.consecutive_loss,
            });
            
            // Keep only recent events (last 10 seconds)
            let cutoff = Instant::now() - Duration::from_secs(10);
            while let Some(event) = self.loss_history.front() {
                if event.timestamp < cutoff {
                    self.loss_history.pop_front();
                } else {
                    break;
                }
            }
        }
    }
    
    fn name(&self) -> &'static str {
        "LossFeedbackGenerator"
    }
}

/// Congestion-based feedback generator
/// Generates REMB packets based on bandwidth estimation
pub struct CongestionFeedbackGenerator {
    /// Bandwidth estimation history
    bandwidth_history: VecDeque<BandwidthSample>,
    
    /// Last REMB generation time
    last_remb: Option<Instant>,
    
    /// Current estimated bandwidth
    estimated_bandwidth: u32,
    
    /// Bandwidth trend (increasing/decreasing)
    bandwidth_trend: BandwidthTrend,
    
    /// Confidence in bandwidth estimation (0.0 - 1.0)
    confidence: f32,
}

#[derive(Debug, Clone)]
struct BandwidthSample {
    timestamp: Instant,
    bandwidth_bps: u32,
    rtt_ms: u32,
    loss_rate: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum BandwidthTrend {
    Increasing,
    Stable,
    Decreasing,
    Unknown,
}

impl CongestionFeedbackGenerator {
    /// Create a new congestion-based feedback generator
    pub fn new() -> Self {
        Self {
            bandwidth_history: VecDeque::new(),
            last_remb: None,
            estimated_bandwidth: 1_000_000,  // Start with 1 Mbps estimate
            bandwidth_trend: BandwidthTrend::Unknown,
            confidence: 0.5,  // Moderate initial confidence
        }
    }
    
    /// Update bandwidth estimation based on network metrics
    fn update_bandwidth_estimate(&mut self, rtt_ms: u32, loss_rate: f32, jitter: u32) {
        // Simple bandwidth estimation algorithm
        // In a real implementation, this would use Google Congestion Control or similar
        
        let congestion_factor = self.calculate_congestion_factor(rtt_ms, loss_rate, jitter);
        
        // Adjust bandwidth based on congestion
        if congestion_factor > 0.8 {
            // High congestion - reduce bandwidth
            self.estimated_bandwidth = (self.estimated_bandwidth as f32 * 0.85) as u32;
            self.bandwidth_trend = BandwidthTrend::Decreasing;
            self.confidence = 0.8;  // High confidence in reduction
        } else if congestion_factor < 0.3 {
            // Low congestion - probe for more bandwidth
            self.estimated_bandwidth = (self.estimated_bandwidth as f32 * 1.05) as u32;
            self.bandwidth_trend = BandwidthTrend::Increasing;
            self.confidence = 0.6;  // Moderate confidence in increase
        } else {
            // Stable conditions
            self.bandwidth_trend = BandwidthTrend::Stable;
            self.confidence = 0.9;  // High confidence in stability
        }
        
        // Clamp bandwidth to reasonable limits
        self.estimated_bandwidth = self.estimated_bandwidth.clamp(64_000, 50_000_000);
        
        // Record sample
        self.bandwidth_history.push_back(BandwidthSample {
            timestamp: Instant::now(),
            bandwidth_bps: self.estimated_bandwidth,
            rtt_ms,
            loss_rate,
        });
        
        // Keep only recent samples (last 30 seconds)
        let cutoff = Instant::now() - Duration::from_secs(30);
        while let Some(sample) = self.bandwidth_history.front() {
            if sample.timestamp < cutoff {
                self.bandwidth_history.pop_front();
            } else {
                break;
            }
        }
    }
    
    /// Calculate congestion factor (0.0 = no congestion, 1.0 = severe congestion)
    fn calculate_congestion_factor(&self, rtt_ms: u32, loss_rate: f32, jitter: u32) -> f32 {
        // Weight factors
        let rtt_weight = 0.4;
        let loss_weight = 0.4;
        let jitter_weight = 0.2;
        
        // Normalize metrics
        let normalized_rtt = (rtt_ms as f32 / 500.0).clamp(0.0, 1.0);       // 500ms = max
        let normalized_loss = (loss_rate * 20.0).clamp(0.0, 1.0);          // 5% = max
        let normalized_jitter = (jitter as f32 / 50.0).clamp(0.0, 1.0);    // 50 units = max
        
        normalized_rtt * rtt_weight + 
        normalized_loss * loss_weight + 
        normalized_jitter * jitter_weight
    }
    
    /// Check if we should generate REMB
    fn should_generate_remb(&self, config: &FeedbackConfig) -> bool {
        if !config.enable_remb {
            return false;
        }
        
        // Generate REMB every 2 seconds or on significant bandwidth changes
        if let Some(last) = self.last_remb {
            let elapsed = last.elapsed().as_millis();
            
            // Always generate after 2 seconds
            if elapsed > 2000 {
                return true;
            }
            
            // Generate on significant changes with high confidence
            if self.confidence > 0.7 && elapsed > 500 {
                match self.bandwidth_trend {
                    BandwidthTrend::Increasing | BandwidthTrend::Decreasing => true,
                    _ => false,
                }
            } else {
                false
            }
        } else {
            true  // First REMB
        }
    }
}

impl FeedbackGenerator for CongestionFeedbackGenerator {
    fn generate_feedback(&self, _context: &FeedbackContext, config: &FeedbackConfig) -> Result<FeedbackDecision> {
        if self.should_generate_remb(config) {
            Ok(FeedbackDecision::Remb {
                bitrate_bps: self.estimated_bandwidth,
                confidence: self.confidence,
            })
        } else {
            Ok(FeedbackDecision::None)
        }
    }
    
    fn update_statistics(&mut self, stats: &StreamStats) {
        // Extract network metrics from stats
        let rtt_ms = stats.rtt_ms.unwrap_or(100.0) as u32;  // Default 100ms
        let loss_rate = if stats.packet_count > 0 {
            stats.packets_lost as f32 / stats.packet_count as f32
        } else {
            0.0
        };
        let jitter = stats.jitter_ms as u32;
        
        self.update_bandwidth_estimate(rtt_ms, loss_rate, jitter);
    }
    
    fn name(&self) -> &'static str {
        "CongestionFeedbackGenerator"
    }
}

/// Quality-based feedback generator
/// Combines multiple quality metrics for comprehensive feedback decisions
pub struct QualityFeedbackGenerator {
    /// Quality score history
    quality_history: VecDeque<QualityScore>,
    
    /// Last feedback generation time
    last_feedback: Option<Instant>,
    
    /// Quality trend analysis
    quality_trend: QualityTrend,
    
    /// Thresholds for quality degradation
    quality_thresholds: QualityThresholds,
}

#[derive(Debug, Clone)]
struct QualityScore {
    timestamp: Instant,
    overall_score: f32,  // 0.0 - 1.0, where 1.0 is perfect quality
    loss_score: f32,
    jitter_score: f32,
    bandwidth_score: f32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum QualityTrend {
    Improving,
    Stable,
    Degrading,
    Unknown,
}

#[derive(Debug, Clone)]
struct QualityThresholds {
    excellent: f32,  // > 0.9
    good: f32,       // > 0.7
    fair: f32,       // > 0.5
    poor: f32,       // > 0.3
    // Below 0.3 is considered critical
}

impl Default for QualityThresholds {
    fn default() -> Self {
        Self {
            excellent: 0.9,
            good: 0.7,
            fair: 0.5,
            poor: 0.3,
        }
    }
}

impl QualityFeedbackGenerator {
    /// Create a new quality-based feedback generator
    pub fn new() -> Self {
        Self {
            quality_history: VecDeque::new(),
            last_feedback: None,
            quality_trend: QualityTrend::Unknown,
            quality_thresholds: QualityThresholds::default(),
        }
    }
    
    /// Calculate overall quality score
    fn calculate_quality_score(&self, loss_rate: f32, jitter: u32, bandwidth_ratio: f32) -> QualityScore {
        // Individual component scores (1.0 = perfect, 0.0 = terrible)
        let loss_score = (1.0 - (loss_rate * 10.0)).clamp(0.0, 1.0);  // 10% loss = 0 score
        let jitter_score = (1.0 - (jitter as f32 / 100.0)).clamp(0.0, 1.0);  // 100 jitter = 0 score
        let bandwidth_score = bandwidth_ratio.clamp(0.0, 1.0);  // 1.0 = sufficient bandwidth
        
        // Weighted overall score
        let overall_score = (loss_score * 0.4 + jitter_score * 0.3 + bandwidth_score * 0.3).clamp(0.0, 1.0);
        
        QualityScore {
            timestamp: Instant::now(),
            overall_score,
            loss_score,
            jitter_score,
            bandwidth_score,
        }
    }
    
    /// Determine quality trend based on recent history
    fn update_quality_trend(&mut self) {
        if self.quality_history.len() < 3 {
            self.quality_trend = QualityTrend::Unknown;
            return;
        }
        
        let recent: Vec<f32> = self.quality_history
            .iter()
            .rev()
            .take(3)
            .map(|q| q.overall_score)
            .collect();
        
        if recent[0] > recent[1] && recent[1] > recent[2] {
            self.quality_trend = QualityTrend::Improving;
        } else if recent[0] < recent[1] && recent[1] < recent[2] {
            self.quality_trend = QualityTrend::Degrading;
        } else {
            self.quality_trend = QualityTrend::Stable;
        }
    }
    
    /// Determine appropriate feedback based on quality
    fn determine_feedback(&self, current_quality: &QualityScore) -> FeedbackDecision {
        let score = current_quality.overall_score;
        
        match score {
            s if s < self.quality_thresholds.poor => {
                // Critical quality - request keyframe
                if current_quality.loss_score < 0.3 {
                    FeedbackDecision::Fir {
                        priority: FeedbackPriority::Critical,
                        sequence_number: 1,  // Would be managed by state
                    }
                } else {
                    FeedbackDecision::Pli {
                        priority: FeedbackPriority::Critical,
                        reason: QualityDegradation::PacketLoss {
                            rate: ((1.0 - current_quality.loss_score) * 100.0) as u8,
                            consecutive: 0,  // Would be tracked separately
                        },
                    }
                }
            }
            s if s < self.quality_thresholds.fair => {
                // Poor quality - request picture refresh
                FeedbackDecision::Pli {
                    priority: FeedbackPriority::High,
                    reason: QualityDegradation::PacketLoss {
                        rate: ((1.0 - current_quality.loss_score) * 100.0) as u8,
                        consecutive: 0,
                    },
                }
            }
            s if s < self.quality_thresholds.good => {
                // Fair quality - suggest bandwidth adjustment
                let estimated_bandwidth = 1_000_000;  // Would calculate based on current conditions
                FeedbackDecision::Remb {
                    bitrate_bps: estimated_bandwidth,
                    confidence: 0.7,
                }
            }
            _ => {
                // Good quality - no immediate feedback needed
                FeedbackDecision::None
            }
        }
    }
}

impl FeedbackGenerator for QualityFeedbackGenerator {
    fn generate_feedback(&self, _context: &FeedbackContext, _config: &FeedbackConfig) -> Result<FeedbackDecision> {
        if let Some(latest_quality) = self.quality_history.back() {
            // Only generate feedback if quality is degrading or already poor
            match self.quality_trend {
                QualityTrend::Degrading | QualityTrend::Unknown => {
                    Ok(self.determine_feedback(latest_quality))
                }
                _ => {
                    if latest_quality.overall_score < self.quality_thresholds.fair {
                        Ok(self.determine_feedback(latest_quality))
                    } else {
                        Ok(FeedbackDecision::None)
                    }
                }
            }
        } else {
            Ok(FeedbackDecision::None)
        }
    }
    
    fn update_statistics(&mut self, stats: &StreamStats) {
        // Calculate quality metrics
        let loss_rate = if stats.packet_count > 0 {
            stats.packets_lost as f32 / stats.packet_count as f32
        } else {
            0.0
        };
        
        let bandwidth_ratio = 1.0;  // Would calculate based on available vs required bandwidth
        
        let quality_score = self.calculate_quality_score(loss_rate, stats.jitter_ms as u32, bandwidth_ratio);
        
        self.quality_history.push_back(quality_score);
        
        // Keep only recent quality scores (last 60 seconds)
        let cutoff = Instant::now() - Duration::from_secs(60);
        while let Some(score) = self.quality_history.front() {
            if score.timestamp < cutoff {
                self.quality_history.pop_front();
            } else {
                break;
            }
        }
        
        self.update_quality_trend();
    }
    
    fn name(&self) -> &'static str {
        "QualityFeedbackGenerator"
    }
}

/// Comprehensive feedback generator that combines all strategies
pub struct ComprehensiveFeedbackGenerator {
    loss_generator: LossFeedbackGenerator,
    congestion_generator: CongestionFeedbackGenerator,
    quality_generator: QualityFeedbackGenerator,
    
    /// Last feedback generation time for rate limiting
    last_feedback: Option<Instant>,
}

impl ComprehensiveFeedbackGenerator {
    /// Create a new comprehensive feedback generator
    pub fn new() -> Self {
        Self {
            loss_generator: LossFeedbackGenerator::new(),
            congestion_generator: CongestionFeedbackGenerator::new(),
            quality_generator: QualityFeedbackGenerator::new(),
            last_feedback: None,
        }
    }
    
    /// Combine multiple feedback decisions into a prioritized result
    fn combine_feedback_decisions(&self, decisions: Vec<FeedbackDecision>) -> FeedbackDecision {
        let mut high_priority = Vec::new();
        let mut normal_priority = Vec::new();
        
        for decision in decisions {
            match decision {
                FeedbackDecision::None => continue,
                FeedbackDecision::Pli { priority, .. } | 
                FeedbackDecision::Fir { priority, .. } => {
                    if priority >= FeedbackPriority::High {
                        high_priority.push(decision);
                    } else {
                        normal_priority.push(decision);
                    }
                }
                _ => normal_priority.push(decision),
            }
        }
        
        // Return highest priority feedback, or combine if multiple
        if !high_priority.is_empty() {
            if high_priority.len() == 1 {
                high_priority.into_iter().next().unwrap()
            } else {
                FeedbackDecision::Multiple(high_priority)
            }
        } else if !normal_priority.is_empty() {
            if normal_priority.len() == 1 {
                normal_priority.into_iter().next().unwrap()
            } else {
                FeedbackDecision::Multiple(normal_priority)
            }
        } else {
            FeedbackDecision::None
        }
    }
}

impl FeedbackGenerator for ComprehensiveFeedbackGenerator {
    fn generate_feedback(&self, context: &FeedbackContext, config: &FeedbackConfig) -> Result<FeedbackDecision> {
        // Check rate limiting
        if let Some(last) = self.last_feedback {
            let interval_ms = 1000 / config.max_feedback_rate;  // Convert rate to interval
            if last.elapsed().as_millis() < interval_ms as u128 {
                return Ok(FeedbackDecision::None);
            }
        }
        
        // Collect feedback from all generators
        let mut decisions = Vec::new();
        
        decisions.push(self.loss_generator.generate_feedback(context, config)?);
        decisions.push(self.congestion_generator.generate_feedback(context, config)?);
        decisions.push(self.quality_generator.generate_feedback(context, config)?);
        
        Ok(self.combine_feedback_decisions(decisions))
    }
    
    fn update_statistics(&mut self, stats: &StreamStats) {
        self.loss_generator.update_statistics(stats);
        self.congestion_generator.update_statistics(stats);
        self.quality_generator.update_statistics(stats);
    }
    
    fn name(&self) -> &'static str {
        "ComprehensiveFeedbackGenerator"
    }
} 