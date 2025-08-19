//! Core Feedback Algorithms (moved from rtp-core)
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

/// Over-use detector for bandwidth estimation
struct OverUseDetector {
    /// Threshold for over-use detection
    threshold: f64,
    
    /// Current detector state
    state: OverUseState,
    
    /// Consecutive over-use count
    overuse_count: u32,
    
    /// Consecutive normal count
    normal_count: u32,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum OverUseState {
    Normal,
    OverUsing,
    UnderUsing,
}

impl OverUseDetector {
    fn new() -> Self {
        Self {
            threshold: 12.5,  // ms
            state: OverUseState::Normal,
            overuse_count: 0,
            normal_count: 0,
        }
    }
    
    /// Detect over-use based on arrival time filter
    fn detect(&mut self, filtered_value: f64, variance: f64) -> OverUseState {
        let threshold_with_variance = self.threshold + variance.sqrt() * 2.0;
        
        if filtered_value > threshold_with_variance {
            self.overuse_count += 1;
            self.normal_count = 0;
            
            if self.overuse_count > 3 {
                self.state = OverUseState::OverUsing;
            }
        } else if filtered_value < -threshold_with_variance {
            self.normal_count += 1;
            self.overuse_count = 0;
            
            if self.normal_count > 3 {
                self.state = OverUseState::UnderUsing;
            }
        } else {
            self.normal_count += 1;
            self.overuse_count = 0;
            
            if self.normal_count > 5 {
                self.state = OverUseState::Normal;
            }
        }
        
        self.state
    }
}

/// Remote rate controller for bandwidth management
struct RemoteRateController {
    /// Current bitrate (bps)
    current_bitrate: u32,
    
    /// Maximum bitrate (bps)
    max_bitrate: u32,
    
    /// Minimum bitrate (bps)
    min_bitrate: u32,
    
    /// Increase factor
    increase_factor: f64,
    
    /// Decrease factor
    decrease_factor: f64,
}

impl RemoteRateController {
    fn new() -> Self {
        Self {
            current_bitrate: 300_000,  // Start at 300 kbps
            max_bitrate: 10_000_000,   // 10 Mbps max
            min_bitrate: 30_000,       // 30 kbps min
            increase_factor: 1.05,      // 5% increase
            decrease_factor: 0.85,      // 15% decrease
        }
    }
    
    /// Update bitrate based on congestion state
    fn update(&mut self, state: GccState) -> u32 {
        match state {
            GccState::Increase => {
                self.current_bitrate = ((self.current_bitrate as f64 * self.increase_factor) as u32)
                    .min(self.max_bitrate);
            }
            GccState::Decrease => {
                self.current_bitrate = ((self.current_bitrate as f64 * self.decrease_factor) as u32)
                    .max(self.min_bitrate);
            }
            GccState::Hold => {
                // No change
            }
        }
        
        self.current_bitrate
    }
}

impl GoogleCongestionControl {
    /// Create a new Google Congestion Control instance
    pub fn new() -> Self {
        Self {
            state: GccState::Hold,
            arrival_filter: ArrivalTimeFilter::new(),
            overuse_detector: OverUseDetector::new(),
            rate_controller: RemoteRateController::new(),
            current_estimate: 300_000,  // Start at 300 kbps
            last_update: None,
        }
    }
    
    /// Process new packet arrival
    pub fn on_packet_arrival(&mut self, timestamp: Instant, size_bytes: usize) -> u32 {
        if let Some(last) = self.last_update {
            let delta_ms = last.elapsed().as_millis() as i64;
            
            // Update arrival time filter
            self.arrival_filter.update(delta_ms);
            
            // Detect over-use
            let overuse_state = self.overuse_detector.detect(
                self.arrival_filter.value(),
                self.arrival_filter.variance(),
            );
            
            // Update GCC state based on over-use detection
            self.state = match overuse_state {
                OverUseState::OverUsing => GccState::Decrease,
                OverUseState::UnderUsing => GccState::Increase,
                OverUseState::Normal => GccState::Hold,
            };
            
            // Update bitrate estimate
            self.current_estimate = self.rate_controller.update(self.state);
        }
        
        self.last_update = Some(timestamp);
        self.current_estimate
    }
    
    /// Get current bandwidth estimate
    pub fn get_estimate(&self) -> u32 {
        self.current_estimate
    }
    
    /// Get current congestion state
    pub fn get_state(&self) -> GccState {
        self.state
    }
}

impl Default for GoogleCongestionControl {
    fn default() -> Self {
        Self::new()
    }
}

/// Calculate Mean Opinion Score (MOS) based on R-factor
/// 
/// This implements the ITU-T G.107 E-model for calculating MOS from R-factor.
pub fn calculate_mos_from_rfactor(r_factor: f32) -> f32 {
    if r_factor < 0.0 {
        return 1.0;
    } else if r_factor > 100.0 {
        return 4.5;
    }
    
    // MOS calculation according to ITU-T G.107
    if r_factor < 0.0 {
        1.0
    } else if r_factor < 6.52 {
        1.0
    } else if r_factor < 100.0 {
        1.0 + 0.035 * r_factor + 0.000007 * r_factor * (r_factor - 60.0) * (100.0 - r_factor)
    } else {
        4.5
    }
}

/// Calculate R-factor based on network parameters
///
/// The R-factor is a transmission quality rating factor used in the E-model.
pub fn calculate_rfactor(
    delay_ms: f32,
    packet_loss_percent: f32,
    jitter_ms: f32,
) -> f32 {
    // Base R-factor (perfect conditions)
    let mut r_factor = 93.2;
    
    // Delay impairment (Id)
    let id = if delay_ms < 100.0 {
        0.0
    } else if delay_ms < 400.0 {
        0.024 * delay_ms + 0.11 * (delay_ms - 177.3).max(0.0)
    } else {
        0.024 * delay_ms + 0.11 * (delay_ms - 177.3)
    };
    
    // Equipment impairment (Ie)
    let ie = 30.0 * packet_loss_percent;
    
    // Additional jitter impairment
    let jitter_impairment = if jitter_ms > 20.0 {
        (jitter_ms - 20.0) * 0.5
    } else {
        0.0
    };
    
    // Calculate final R-factor
    r_factor -= id + ie + jitter_impairment;
    
    r_factor.max(0.0).min(100.0)
}

/// Transport-wide congestion control feedback processor
pub struct TransportCcProcessor {
    /// Packet arrival times indexed by sequence number
    arrival_times: VecDeque<(u16, Instant)>,
    
    /// Last feedback generation time
    last_feedback_time: Option<Instant>,
    
    /// Feedback interval
    feedback_interval: Duration,
    
    /// Base sequence number for current feedback
    base_sequence: u16,
}

impl TransportCcProcessor {
    /// Create a new transport-wide CC processor
    pub fn new() -> Self {
        Self {
            arrival_times: VecDeque::new(),
            last_feedback_time: None,
            feedback_interval: Duration::from_millis(100),  // 100ms default
            base_sequence: 0,
        }
    }
    
    /// Record packet arrival
    pub fn on_packet_arrival(&mut self, sequence: u16, arrival_time: Instant) {
        // Add to arrival times
        self.arrival_times.push_back((sequence, arrival_time));
        
        // Keep only recent arrivals (last 5 seconds)
        let cutoff = Instant::now() - Duration::from_secs(5);
        while let Some((_, time)) = self.arrival_times.front() {
            if *time < cutoff {
                self.arrival_times.pop_front();
            } else {
                break;
            }
        }
    }
    
    /// Check if feedback should be sent
    pub fn should_send_feedback(&self) -> bool {
        match self.last_feedback_time {
            None => !self.arrival_times.is_empty(),
            Some(last) => last.elapsed() >= self.feedback_interval && !self.arrival_times.is_empty(),
        }
    }
    
    /// Generate transport-wide CC feedback data
    pub fn generate_feedback(&mut self) -> Option<TransportCcFeedback> {
        if !self.should_send_feedback() {
            return None;
        }
        
        if self.arrival_times.is_empty() {
            return None;
        }
        
        // Get base sequence and reference time
        let (base_seq, ref_time) = self.arrival_times[0];
        
        // Build packet status list
        let mut packet_status = Vec::new();
        
        for &(seq, arrival) in &self.arrival_times {
            let delta = arrival.duration_since(ref_time).as_micros() as i32;
            packet_status.push(PacketStatus {
                sequence_number: seq,
                arrival_time_delta: delta,
            });
        }
        
        self.last_feedback_time = Some(Instant::now());
        self.base_sequence = base_seq;
        
        Some(TransportCcFeedback {
            base_sequence: base_seq,
            packet_status_count: packet_status.len() as u16,
            reference_time: ref_time,
            packet_status,
        })
    }
}

impl Default for TransportCcProcessor {
    fn default() -> Self {
        Self::new()
    }
}

/// Transport-wide congestion control feedback structure
#[derive(Debug, Clone)]
pub struct TransportCcFeedback {
    /// Base sequence number
    pub base_sequence: u16,
    
    /// Number of packet status entries
    pub packet_status_count: u16,
    
    /// Reference time
    pub reference_time: Instant,
    
    /// Packet status list
    pub packet_status: Vec<PacketStatus>,
}

/// Packet status for transport-wide CC
#[derive(Debug, Clone)]
pub struct PacketStatus {
    /// Sequence number
    pub sequence_number: u16,
    
    /// Arrival time delta in microseconds from reference
    pub arrival_time_delta: i32,
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_mos_calculation() {
        // Test boundary conditions
        assert_eq!(calculate_mos_from_rfactor(-10.0), 1.0);
        assert_eq!(calculate_mos_from_rfactor(110.0), 4.5);
        
        // Test typical R-factor values
        let mos_90 = calculate_mos_from_rfactor(90.0);
        assert!(mos_90 > 4.0 && mos_90 < 4.5);
        
        let mos_70 = calculate_mos_from_rfactor(70.0);
        assert!(mos_70 > 3.5 && mos_70 < 4.0);
        
        let mos_50 = calculate_mos_from_rfactor(50.0);
        assert!(mos_50 > 2.5 && mos_50 < 3.5);
    }
    
    #[test]
    fn test_rfactor_calculation() {
        // Perfect conditions
        let r_perfect = calculate_rfactor(0.0, 0.0, 0.0);
        assert!(r_perfect > 90.0);
        
        // High delay
        let r_delay = calculate_rfactor(300.0, 0.0, 0.0);
        assert!(r_delay < 85.0);
        
        // High packet loss
        let r_loss = calculate_rfactor(50.0, 2.0, 0.0);
        assert!(r_loss < 70.0);
        
        // High jitter
        let r_jitter = calculate_rfactor(50.0, 0.0, 50.0);
        assert!(r_jitter < 85.0);
    }
    
    #[test]
    fn test_google_congestion_control() {
        let mut gcc = GoogleCongestionControl::new();
        
        // Initial estimate
        assert_eq!(gcc.get_estimate(), 300_000);
        
        // Simulate packet arrivals
        let now = Instant::now();
        let estimate1 = gcc.on_packet_arrival(now, 1500);
        assert_eq!(estimate1, 300_000);
        
        // Second packet after delay
        let estimate2 = gcc.on_packet_arrival(now + Duration::from_millis(20), 1500);
        assert!(estimate2 > 0);
    }
}