//! RTP Statistics Module
//!
//! This module provides mechanisms for collecting and analyzing RTP session statistics
//! including packet loss, jitter, round-trip time, and other metrics defined in RFC 3550.

pub mod jitter;
pub mod loss;
pub mod rtt;
pub mod reports;

pub use jitter::JitterEstimator;
pub use loss::{PacketLossTracker, PacketLossStats, PacketLossResult};
pub use rtt::{RttEstimator, RttStats};
pub use reports::{RtcpReportGenerator, RTCP_MIN_INTERVAL, RTCP_BANDWIDTH_FRACTION};

use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use crate::RtpSequenceNumber;
use crate::packet::rtcp::NtpTimestamp;

/// RTP packet statistics
#[derive(Debug, Clone, Default)]
pub struct RtpStats {
    /// Total number of RTP packets sent
    pub packets_sent: u64,
    
    /// Total number of RTP bytes sent
    pub bytes_sent: u64,
    
    /// Total number of RTP packets received
    pub packets_received: u64,
    
    /// Total number of RTP bytes received
    pub bytes_received: u64,
    
    /// Packets lost (based on sequence numbers)
    pub packets_lost: u64,
    
    /// Fraction of packets lost since last report (0-255 scale where 255 = 100%)
    pub fraction_lost: u8,
    
    /// Duplicate packets received
    pub packets_duplicated: u64,
    
    /// Out-of-order packets received
    pub packets_out_of_order: u64,
    
    /// Interarrival jitter (in RTP timestamp units)
    pub jitter: f64,
    
    /// Round-trip time (in milliseconds)
    pub round_trip_time_ms: Option<f64>,
    
    /// Last sequence number received
    pub last_seq: Option<RtpSequenceNumber>,
    
    /// Estimated highest sequence number
    pub highest_seq: u32,
    
    /// First sequence number received (base sequence)
    pub base_seq: Option<RtpSequenceNumber>,
    
    /// Last SR timestamp received
    pub last_sr_timestamp: Option<NtpTimestamp>,
    
    /// Delay since last SR (in milliseconds)
    pub delay_since_last_sr_ms: Option<u32>,
}

/// Comprehensive RTP statistics manager integrating all statistical components
pub struct RtpStatsManager {
    /// Overall session statistics
    stats: Arc<Mutex<RtpStats>>,
    
    /// Jitter estimator for accurate jitter calculations
    jitter_estimator: JitterEstimator,
    
    /// Packet loss tracker
    loss_tracker: PacketLossTracker,
    
    /// RTT estimator
    rtt_estimator: RttEstimator,
    
    /// RTCP report generator
    rtcp_generator: Option<RtcpReportGenerator>,
    
    /// Time of last stats reset
    start_time: Instant,
    
    /// Clock rate for timestamp conversions
    clock_rate: u32,
}

impl RtpStatsManager {
    /// Create a new RTP statistics manager
    pub fn new(clock_rate: u32) -> Self {
        Self {
            stats: Arc::new(Mutex::new(RtpStats::default())),
            jitter_estimator: JitterEstimator::new(clock_rate),
            loss_tracker: PacketLossTracker::new(),
            rtt_estimator: RttEstimator::new(),
            rtcp_generator: None,
            start_time: Instant::now(),
            clock_rate,
        }
    }
    
    /// Create a new RTP statistics manager with RTCP support
    pub fn new_with_rtcp(clock_rate: u32, local_ssrc: u32, cname: String) -> Self {
        let mut manager = Self::new(clock_rate);
        manager.rtcp_generator = Some(RtcpReportGenerator::new(local_ssrc, cname));
        manager
    }
    
    /// Get a copy of the current statistics
    pub fn get_stats(&self) -> RtpStats {
        self.stats.lock().unwrap().clone()
    }
    
    /// Reset all statistics
    pub fn reset(&mut self) {
        *self.stats.lock().unwrap() = RtpStats::default();
        self.jitter_estimator.reset();
        self.loss_tracker.reset();
        self.rtt_estimator.reset();
        self.start_time = Instant::now();
    }
    
    /// Get the duration since start or last reset
    pub fn duration(&self) -> Duration {
        self.start_time.elapsed()
    }
    
    /// Update statistics for a sent packet
    pub fn update_sent(&mut self, bytes: usize) {
        let mut stats = self.stats.lock().unwrap();
        stats.packets_sent += 1;
        stats.bytes_sent += bytes as u64;
        
        // Update RTCP generator if available
        if let Some(generator) = &mut self.rtcp_generator {
            generator.update_sent_stats(1, bytes as u32);
        }
    }
    
    /// Update statistics for a received packet
    pub fn update_received(&mut self, seq: RtpSequenceNumber, timestamp: u32, bytes: usize, arrival_time: Instant) {
        let mut stats = self.stats.lock().unwrap();
        
        // Update basic counters
        stats.packets_received += 1;
        stats.bytes_received += bytes as u64;
        
        // Process packet loss
        let result = self.loss_tracker.process(seq);
        
        // Update loss statistics based on the result
        match result {
            PacketLossResult::FirstPacket { seq } => {
                stats.base_seq = Some(seq);
                stats.highest_seq = seq as u32;
                stats.last_seq = Some(seq);
            },
            PacketLossResult::Sequential { seq } => {
                stats.highest_seq = seq as u32;
                stats.last_seq = Some(seq);
            },
            PacketLossResult::Gap { seq, expected, lost } => {
                stats.packets_lost += lost as u64;
                stats.highest_seq = seq as u32;
                stats.last_seq = Some(seq);
            },
            PacketLossResult::Duplicate { seq } => {
                stats.packets_duplicated += 1;
            },
            PacketLossResult::Reordered { seq, expected } => {
                stats.packets_out_of_order += 1;
                stats.last_seq = Some(seq);
            },
            PacketLossResult::Unknown => {},
        }
        
        // Update jitter calculation
        let jitter = self.jitter_estimator.update(timestamp, arrival_time);
        stats.jitter = jitter;
        
        // Update fraction lost from loss tracker
        let loss_stats = self.loss_tracker.get_stats();
        stats.fraction_lost = loss_stats.fraction_lost;
        
        // Update RTCP generator if available
        if let Some(generator) = &mut self.rtcp_generator {
            generator.process_received_packet(0, seq); // SSRC would be extracted from the packet
        }
    }
    
    /// Update round-trip time
    pub fn update_rtt(&self, rtt_ms: f64) {
        let mut stats = self.stats.lock().unwrap();
        stats.round_trip_time_ms = Some(rtt_ms);
    }
    
    /// Update RTCP SR information
    pub fn update_sr_info(&self, last_sr: NtpTimestamp, delay_ms: u32) {
        let mut stats = self.stats.lock().unwrap();
        stats.last_sr_timestamp = Some(last_sr);
        stats.delay_since_last_sr_ms = Some(delay_ms);
    }
    
    /// Get the RTCP report generator if available
    pub fn rtcp_generator(&mut self) -> Option<&mut RtcpReportGenerator> {
        self.rtcp_generator.as_mut()
    }
    
    /// Get the jitter estimator
    pub fn jitter_estimator(&self) -> &JitterEstimator {
        &self.jitter_estimator
    }
    
    /// Get the loss tracker
    pub fn loss_tracker(&self) -> &PacketLossTracker {
        &self.loss_tracker
    }
    
    /// Get the RTT estimator
    pub fn rtt_estimator(&self) -> &RttEstimator {
        &self.rtt_estimator
    }
}

impl Default for RtpStatsManager {
    fn default() -> Self {
        Self::new(8000) // Default 8kHz clock rate
    }
}

/// Check if sequence 'a' is older than sequence 'b', handling wraparound
fn is_sequence_older(a: RtpSequenceNumber, b: RtpSequenceNumber) -> bool {
    if a == b {
        return false; // A sequence is not older than itself
    }
    
    // Compare with wraparound as per RFC 3550
    // A sequence number is considered older if it's in the first half of the sequence
    // space behind the other sequence number.
    // This handles the case where 65000 is newer than 1000 but 1000 is newer than 33000
    let half_range = 0x8000;
    (b > a && b - a < half_range) || (a > b && a - b >= half_range)
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_sequence_comparison() {
        // Normal cases
        assert!(is_sequence_older(100, 101));
        assert!(is_sequence_older(100, 200));
        assert!(!is_sequence_older(200, 100));
        assert!(!is_sequence_older(101, 100));
        
        // Wraparound cases
        assert!(is_sequence_older(65530, 10));
        assert!(!is_sequence_older(10, 65530));
        
        // Edge cases
        assert!(!is_sequence_older(100, 100));
        assert!(!is_sequence_older(0, 32768)); // 32768 is 0x8000 - a sequence is only older if diff < 0x8000
        assert!(is_sequence_older(32768, 0));
    }
    
    #[test]
    fn test_stats_manager() {
        let mut manager = RtpStatsManager::new(8000);
        
        // Test initial state
        let stats = manager.get_stats();
        assert_eq!(stats.packets_sent, 0);
        assert_eq!(stats.packets_received, 0);
        assert_eq!(stats.packets_lost, 0);
        assert_eq!(stats.packets_duplicated, 0);
        assert_eq!(stats.packets_out_of_order, 0);
        assert!(stats.last_seq.is_none());
        
        // Test updating sent
        manager.update_sent(100);
        let stats = manager.get_stats();
        assert_eq!(stats.packets_sent, 1);
        assert_eq!(stats.bytes_sent, 100);
    }
} 