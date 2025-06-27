//! Core Feedback Algorithms
//!
//! This module implements the core algorithms used for bandwidth estimation,
//! congestion detection, and quality assessment in RTCP feedback generation.

use std::time::{Instant, Duration};
use std::collections::VecDeque;

/// Google Congestion Control (GCC) implementation
/// Based on the WebRTC implementation for bandwidth estimation
pub struct GoogleCongestionControl {
    /// State of the congestion control algorithm
    state: GccState,
    
    /// Arrival time filter for calculating inter-arrival times
    arrival_filter: ArrivalTimeFilter,
    
    /// Over-use detector for bandwidth estimation
    overuse_detector: OverUseDetector,
    
    /// Remote rate controller
    rate_controller: RemoteRateController,
    
    /// Current bandwidth estimate (bits per second)
    current_estimate: u32,
    
    /// Last update time
    last_update: Option<Instant>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GccState {
    /// Increase bandwidth estimate
    Increase,
    
    /// Hold current estimate
    Hold,
    
    /// Decrease bandwidth estimate
    Decrease,
}

/// Arrival time filter for calculating inter-arrival time variations
struct ArrivalTimeFilter {
    /// History of arrival time deltas
    deltas: VecDeque<i64>,
    
    /// Current smoothed value
    smoothed_value: f64,
    
    /// Variance estimate
    variance: f64,
}

impl ArrivalTimeFilter {
    fn new() -> Self {
        Self {
            deltas: VecDeque::new(),
            smoothed_value: 0.0,
            variance: 0.0,
        }
    }
    
    /// Update filter with new arrival time delta
    fn update(&mut self, delta_ms: i64) {
        self.deltas.push_back(delta_ms);
        
        // Keep only recent samples (last 60 samples)
        while self.deltas.len() > 60 {
            self.deltas.pop_front();
        }
        
        // Calculate smoothed value and variance
        if !self.deltas.is_empty() {
            let sum: i64 = self.deltas.iter().sum();
            let mean = sum as f64 / self.deltas.len() as f64;
            
            self.smoothed_value = self.smoothed_value * 0.9 + mean * 0.1;
            
            let variance_sum: f64 = self.deltas
                .iter()
                .map(|&x| {
                    let diff = x as f64 - mean;
                    diff * diff
                })
                .sum();
                
            self.variance = variance_sum / self.deltas.len() as f64;
        }
    }
    
    /// Get current filtered value
    fn value(&self) -> f64 {
        self.smoothed_value
    }
    
    /// Get current variance
    fn variance(&self) -> f64 {
        self.variance
    }
}

/// Over-use detector based on Kalman filtering
struct OverUseDetector {
    /// Threshold for over-use detection
    threshold: f64,
    
    /// State estimate
    estimate: f64,
    
    /// Estimation error covariance
    error_covariance: f64,
    
    /// Process noise variance
    process_noise: f64,
    
    /// Measurement noise variance
    measurement_noise: f64,
    
    /// Over-use hypothesis
    hypothesis: OverUseHypothesis,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OverUseHypothesis {
    Normal,
    OverUsing,
    UnderUsing,
}

impl OverUseDetector {
    fn new() -> Self {
        Self {
            threshold: 12.5,  // ms
            estimate: 0.0,
            error_covariance: 100.0,
            process_noise: 1e-3,
            measurement_noise: 1e-1,
            hypothesis: OverUseHypothesis::Normal,
        }
    }
    
    /// Update detector with arrival time delta
    fn update(&mut self, arrival_delta: f64, timestamp_delta: f64) {
        if timestamp_delta <= 0.0 {
            return;
        }
        
        // Prediction step
        self.error_covariance += self.process_noise;
        
        // Update step
        let innovation = arrival_delta - self.estimate;
        let kalman_gain = self.error_covariance / (self.error_covariance + self.measurement_noise);
        
        self.estimate += kalman_gain * innovation;
        self.error_covariance *= (1.0 - kalman_gain);
        
        // Hypothesis testing
        let threshold = self.threshold;
        self.hypothesis = if self.estimate > threshold {
            OverUseHypothesis::OverUsing
        } else if self.estimate < -threshold {
            OverUseHypothesis::UnderUsing
        } else {
            OverUseHypothesis::Normal
        };
    }
    
    /// Get current hypothesis
    fn hypothesis(&self) -> OverUseHypothesis {
        self.hypothesis
    }
    
    /// Get current estimate
    fn estimate(&self) -> f64 {
        self.estimate
    }
}

/// Remote rate controller for bandwidth adjustment
struct RemoteRateController {
    /// Current state
    state: GccState,
    
    /// Current rate (bits per second)
    current_rate: u32,
    
    /// Last update time
    last_update: Instant,
    
    /// Rate increase parameters
    increase_rate: f64,
    
    /// Rate decrease factor
    decrease_factor: f64,
    
    /// Minimum rate
    min_rate: u32,
    
    /// Maximum rate
    max_rate: u32,
}

impl RemoteRateController {
    fn new(initial_rate: u32) -> Self {
        Self {
            state: GccState::Hold,
            current_rate: initial_rate,
            last_update: Instant::now(),
            increase_rate: 1.08,  // 8% increase factor
            decrease_factor: 0.85,  // 15% decrease factor
            min_rate: 30_000,   // 30 kbps minimum
            max_rate: 50_000_000,  // 50 Mbps maximum
        }
    }
    
    /// Update rate based on over-use detector feedback
    fn update(&mut self, hypothesis: OverUseHypothesis, incoming_rate: u32) -> u32 {
        let now = Instant::now();
        let time_delta = now.duration_since(self.last_update).as_millis() as f64;
        self.last_update = now;
        
        match hypothesis {
            OverUseHypothesis::OverUsing => {
                // Decrease rate
                self.state = GccState::Decrease;
                self.current_rate = (self.current_rate as f64 * self.decrease_factor) as u32;
            }
            OverUseHypothesis::UnderUsing => {
                // Increase rate gradually
                self.state = GccState::Increase;
                if time_delta > 100.0 {  // Only increase after 100ms
                    let increase = (incoming_rate as f64 * 0.05).max(1000.0);  // At least 1 kbps
                    self.current_rate = (self.current_rate as f64 + increase) as u32;
                }
            }
            OverUseHypothesis::Normal => {
                // Hold current rate
                self.state = GccState::Hold;
            }
        }
        
        // Clamp to limits
        self.current_rate = self.current_rate.clamp(self.min_rate, self.max_rate);
        self.current_rate
    }
    
    /// Get current rate
    fn current_rate(&self) -> u32 {
        self.current_rate
    }
    
    /// Get current state
    fn state(&self) -> GccState {
        self.state
    }
}

impl GoogleCongestionControl {
    /// Create a new GCC instance
    pub fn new(initial_bitrate: u32) -> Self {
        Self {
            state: GccState::Hold,
            arrival_filter: ArrivalTimeFilter::new(),
            overuse_detector: OverUseDetector::new(),
            rate_controller: RemoteRateController::new(initial_bitrate),
            current_estimate: initial_bitrate,
            last_update: None,
        }
    }
    
    /// Update GCC with packet feedback
    pub fn update_with_feedback(&mut self, packets: &[PacketFeedback]) -> u32 {
        if packets.is_empty() {
            return self.current_estimate;
        }
        
        // Calculate inter-arrival times
        for window in packets.windows(2) {
            let delta_arrival = window[1].arrival_time_ms - window[0].arrival_time_ms;
            let delta_timestamp = window[1].send_time_ms - window[0].send_time_ms;
            
            // Update arrival time filter
            self.arrival_filter.update(delta_arrival);
            
            // Update over-use detector
            self.overuse_detector.update(
                self.arrival_filter.value(),
                delta_timestamp as f64,
            );
        }
        
        // Calculate incoming rate
        let incoming_rate = self.calculate_incoming_rate(packets);
        
        // Update rate controller
        self.current_estimate = self.rate_controller.update(
            self.overuse_detector.hypothesis(),
            incoming_rate,
        );
        
        self.state = self.rate_controller.state();
        self.last_update = Some(Instant::now());
        
        self.current_estimate
    }
    
    /// Calculate incoming packet rate
    fn calculate_incoming_rate(&self, packets: &[PacketFeedback]) -> u32 {
        if packets.len() < 2 {
            return self.current_estimate;
        }
        
        let time_span = packets.last().unwrap().arrival_time_ms - packets[0].arrival_time_ms;
        if time_span <= 0 {
            return self.current_estimate;
        }
        
        let total_size: u32 = packets.iter().map(|p| p.size_bytes).sum();
        let rate_bps = (total_size * 8 * 1000) / time_span as u32;
        
        rate_bps
    }
    
    /// Get current bandwidth estimate
    pub fn current_estimate(&self) -> u32 {
        self.current_estimate
    }
    
    /// Get current state
    pub fn state(&self) -> GccState {
        self.state
    }
    
    /// Get over-use detector estimate
    pub fn overuse_estimate(&self) -> f64 {
        self.overuse_detector.estimate()
    }
}

/// Packet feedback information for GCC
#[derive(Debug, Clone)]
pub struct PacketFeedback {
    /// Packet sequence number
    pub sequence_number: u16,
    
    /// Send time (milliseconds)
    pub send_time_ms: i64,
    
    /// Arrival time (milliseconds)
    pub arrival_time_ms: i64,
    
    /// Packet size in bytes
    pub size_bytes: u32,
}

/// Simple bandwidth estimation algorithm
/// Alternative to GCC for simpler use cases
pub struct SimpleBandwidthEstimator {
    /// Recent throughput samples
    throughput_samples: VecDeque<ThroughputSample>,
    
    /// Current estimate
    current_estimate: u32,
    
    /// Smoothing factor (0.0 - 1.0)
    smoothing_factor: f64,
    
    /// Minimum estimate
    min_estimate: u32,
    
    /// Maximum estimate
    max_estimate: u32,
}

#[derive(Debug, Clone)]
struct ThroughputSample {
    timestamp: Instant,
    bytes_per_second: u32,
    rtt_ms: u32,
    loss_rate: f32,
}

impl SimpleBandwidthEstimator {
    /// Create a new simple bandwidth estimator
    pub fn new(initial_estimate: u32) -> Self {
        Self {
            throughput_samples: VecDeque::new(),
            current_estimate: initial_estimate,
            smoothing_factor: 0.1,
            min_estimate: 64_000,    // 64 kbps
            max_estimate: 100_000_000, // 100 Mbps
        }
    }
    
    /// Update estimate with network metrics
    pub fn update(&mut self, bytes_received: u32, time_window_ms: u32, rtt_ms: u32, loss_rate: f32) {
        if time_window_ms == 0 {
            return;
        }
        
        // Calculate throughput
        let bytes_per_second = (bytes_received * 1000) / time_window_ms;
        
        // Apply congestion adjustment
        let congestion_factor = self.calculate_congestion_factor(rtt_ms, loss_rate);
        let adjusted_throughput = (bytes_per_second as f64 * congestion_factor) as u32;
        
        // Smooth the estimate
        self.current_estimate = (
            self.current_estimate as f64 * (1.0 - self.smoothing_factor) +
            adjusted_throughput as f64 * self.smoothing_factor
        ) as u32;
        
        // Clamp to limits
        self.current_estimate = self.current_estimate.clamp(self.min_estimate, self.max_estimate);
        
        // Record sample
        self.throughput_samples.push_back(ThroughputSample {
            timestamp: Instant::now(),
            bytes_per_second: adjusted_throughput,
            rtt_ms,
            loss_rate,
        });
        
        // Keep only recent samples (last 30 seconds)
        let cutoff = Instant::now() - Duration::from_secs(30);
        while let Some(sample) = self.throughput_samples.front() {
            if sample.timestamp < cutoff {
                self.throughput_samples.pop_front();
            } else {
                break;
            }
        }
    }
    
    /// Calculate congestion factor based on network conditions
    fn calculate_congestion_factor(&self, rtt_ms: u32, loss_rate: f32) -> f64 {
        // Start with no adjustment
        let mut factor = 1.0;
        
        // Adjust for RTT (high RTT indicates congestion)
        if rtt_ms > 100 {
            factor *= 1.0 - ((rtt_ms - 100) as f64 / 1000.0).min(0.5);
        }
        
        // Adjust for loss rate
        if loss_rate > 0.01 {  // 1% loss threshold
            factor *= 1.0 - (loss_rate as f64 * 5.0).min(0.8);
        }
        
        factor.max(0.1)  // Never reduce by more than 90%
    }
    
    /// Get current bandwidth estimate
    pub fn current_estimate(&self) -> u32 {
        self.current_estimate
    }
    
    /// Get confidence in the estimate (0.0 - 1.0)
    pub fn confidence(&self) -> f32 {
        if self.throughput_samples.len() < 3 {
            return 0.3;  // Low confidence with few samples
        }
        
        // Calculate variance in recent samples
        let recent_samples: Vec<u32> = self.throughput_samples
            .iter()
            .rev()
            .take(10)
            .map(|s| s.bytes_per_second)
            .collect();
        
        if recent_samples.len() < 2 {
            return 0.5;
        }
        
        let mean = recent_samples.iter().sum::<u32>() as f64 / recent_samples.len() as f64;
        let variance = recent_samples
            .iter()
            .map(|&x| {
                let diff = x as f64 - mean;
                diff * diff
            })
            .sum::<f64>() / recent_samples.len() as f64;
        
        let coefficient_of_variation = variance.sqrt() / mean;
        
        // High variance = low confidence
        (1.0 - coefficient_of_variation.min(1.0)).max(0.1) as f32
    }
}

/// Quality assessment algorithm
/// Combines multiple metrics into an overall quality score
pub struct QualityAssessment {
    /// Weight for loss rate in quality calculation
    loss_weight: f32,
    
    /// Weight for jitter in quality calculation
    jitter_weight: f32,
    
    /// Weight for RTT in quality calculation
    rtt_weight: f32,
    
    /// Weight for bandwidth utilization in quality calculation
    bandwidth_weight: f32,
}

impl Default for QualityAssessment {
    fn default() -> Self {
        Self {
            loss_weight: 0.4,
            jitter_weight: 0.25,
            rtt_weight: 0.2,
            bandwidth_weight: 0.15,
        }
    }
}

impl QualityAssessment {
    /// Create a new quality assessment with custom weights
    pub fn new(loss_weight: f32, jitter_weight: f32, rtt_weight: f32, bandwidth_weight: f32) -> Self {
        let total = loss_weight + jitter_weight + rtt_weight + bandwidth_weight;
        Self {
            loss_weight: loss_weight / total,
            jitter_weight: jitter_weight / total,
            rtt_weight: rtt_weight / total,
            bandwidth_weight: bandwidth_weight / total,
        }
    }
    
    /// Calculate overall quality score (0.0 - 1.0, where 1.0 is perfect)
    pub fn calculate_quality(&self, metrics: &QualityMetrics) -> f32 {
        // Normalize each metric to 0.0 - 1.0 scale
        let loss_score = (1.0 - (metrics.loss_rate * 20.0)).clamp(0.0, 1.0);  // 5% loss = 0 score
        let jitter_score = (1.0 - (metrics.jitter_ms / 100.0)).clamp(0.0, 1.0);  // 100ms jitter = 0 score
        let rtt_score = (1.0 - (metrics.rtt_ms / 500.0)).clamp(0.0, 1.0);  // 500ms RTT = 0 score
        let bandwidth_score = metrics.bandwidth_utilization.clamp(0.0, 1.0);
        
        // Weighted sum
        loss_score * self.loss_weight +
        jitter_score * self.jitter_weight +
        rtt_score * self.rtt_weight +
        bandwidth_score * self.bandwidth_weight
    }
    
    /// Calculate MOS (Mean Opinion Score) from quality score
    pub fn quality_to_mos(&self, quality_score: f32) -> f32 {
        // Convert 0.0-1.0 quality to 1.0-5.0 MOS scale
        1.0 + quality_score * 4.0
    }
    
    /// Determine if quality degradation requires feedback
    pub fn requires_feedback(&self, quality_score: f32, threshold: f32) -> bool {
        quality_score < threshold
    }
}

/// Quality metrics input for assessment
#[derive(Debug, Clone)]
pub struct QualityMetrics {
    /// Packet loss rate (0.0 - 1.0)
    pub loss_rate: f32,
    
    /// Jitter in milliseconds
    pub jitter_ms: f32,
    
    /// Round-trip time in milliseconds
    pub rtt_ms: f32,
    
    /// Bandwidth utilization ratio (0.0 - 1.0)
    pub bandwidth_utilization: f32,
} 