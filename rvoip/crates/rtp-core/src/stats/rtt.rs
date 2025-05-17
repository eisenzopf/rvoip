use std::collections::VecDeque;
use std::time::{Duration, Instant};

use crate::packet::rtcp::NtpTimestamp;
use crate::RtpSsrc;

/// Round-trip time estimator for RTP/RTCP
#[derive(Debug, Clone)]
pub struct RttEstimator {
    /// Current RTT estimate in seconds
    rtt: f64,
    
    /// Variance of RTT estimate
    rtt_var: f64,
    
    /// History of RTT measurements
    history: VecDeque<f64>,
    
    /// Maximum size of history
    max_history: usize,
    
    /// Minimum RTT seen
    min_rtt: f64,
    
    /// Maximum RTT seen
    max_rtt: f64,
    
    /// Number of RTT samples processed
    samples: u64,
    
    /// Last time a measurement was taken
    last_measurement: Option<Instant>,
    
    /// Map of sent SR timestamps by SSRC
    sr_timestamps: Vec<(RtpSsrc, NtpTimestamp, Instant)>,
    
    /// Maximum age of stored SR timestamps
    max_sr_age: Duration,
}

impl RttEstimator {
    /// Create a new RTT estimator
    pub fn new() -> Self {
        Self {
            rtt: 0.0,
            rtt_var: 0.0,
            history: VecDeque::with_capacity(100),
            max_history: 100,
            min_rtt: f64::MAX,
            max_rtt: 0.0,
            samples: 0,
            last_measurement: None,
            sr_timestamps: Vec::new(),
            max_sr_age: Duration::from_secs(30),
        }
    }
    
    /// Record the time when an SR was sent
    pub fn record_sr_sent(&mut self, ssrc: RtpSsrc, ntp_timestamp: NtpTimestamp) {
        self.sr_timestamps.push((ssrc, ntp_timestamp, Instant::now()));
        
        // Clean up old timestamps
        let now = Instant::now();
        self.sr_timestamps.retain(|(_, _, timestamp)| {
            now.duration_since(*timestamp) < self.max_sr_age
        });
    }
    
    /// Process an RTCP receiver report to update RTT
    pub fn process_receiver_report(
        &mut self,
        ssrc: RtpSsrc,
        last_sr: u32,
        delay_since_last_sr: u32,
    ) -> Option<f64> {
        if last_sr == 0 {
            // No SR reference, can't calculate RTT
            return None;
        }
        
        // Find the corresponding SR record
        // We need to match the NTP timestamp from our sent SRs with the one in the receiver report
        // The last_sr in the receiver report contains the middle 16 bits of the NTP timestamp seconds 
        let sr_record = self.sr_timestamps.iter()
            .find(|(s, ntp, _)| {
                *s == ssrc && ((ntp.seconds >> 16) as u32) == last_sr
            });
        
        if let Some((_, _, sent_time)) = sr_record {
            // Calculate RTT
            let now = Instant::now();
            self.last_measurement = Some(now);
            
            // SR delay from RR (in seconds)
            let delay_seconds = delay_since_last_sr as f64 / 65536.0;
            
            // Full round-trip time: now - sent_time - delay
            let rtt_seconds = now.duration_since(*sent_time).as_secs_f64() - delay_seconds;
            
            // Update RTT using EWMA (Exponentially Weighted Moving Average)
            // Similar to TCP RTT estimation (RFC 6298)
            if self.samples == 0 {
                // First sample
                self.rtt = rtt_seconds;
                self.rtt_var = rtt_seconds / 2.0;
            } else {
                // EWMA update
                const ALPHA: f64 = 0.125; // 1/8
                const BETA: f64 = 0.25;   // 1/4
                
                // Update RTT variance
                let delta = self.rtt - rtt_seconds;
                self.rtt_var = (1.0 - BETA) * self.rtt_var + BETA * delta.abs();
                
                // Update RTT estimate
                self.rtt = (1.0 - ALPHA) * self.rtt + ALPHA * rtt_seconds;
            }
            
            // Update statistics
            self.samples += 1;
            self.min_rtt = self.min_rtt.min(rtt_seconds);
            self.max_rtt = self.max_rtt.max(rtt_seconds);
            
            // Add to history
            if self.history.len() >= self.max_history {
                self.history.pop_front();
            }
            self.history.push_back(rtt_seconds);
            
            Some(rtt_seconds)
        } else {
            None
        }
    }
    
    /// Get the current RTT estimate in seconds
    pub fn get_rtt(&self) -> f64 {
        self.rtt
    }
    
    /// Get the current RTT estimate in milliseconds
    pub fn get_rtt_ms(&self) -> f64 {
        self.rtt * 1000.0
    }
    
    /// Get RTT standard deviation in milliseconds
    pub fn get_rtt_var_ms(&self) -> f64 {
        self.rtt_var * 1000.0
    }
    
    /// Get the minimum RTT seen in milliseconds
    pub fn get_min_rtt_ms(&self) -> f64 {
        if self.min_rtt == f64::MAX {
            0.0
        } else {
            self.min_rtt * 1000.0
        }
    }
    
    /// Get the maximum RTT seen in milliseconds
    pub fn get_max_rtt_ms(&self) -> f64 {
        self.max_rtt * 1000.0
    }
    
    /// Get all RTT statistics
    pub fn get_stats(&self) -> RttStats {
        RttStats {
            rtt_ms: self.get_rtt_ms(),
            rtt_var_ms: self.get_rtt_var_ms(),
            min_rtt_ms: self.get_min_rtt_ms(),
            max_rtt_ms: self.get_max_rtt_ms(),
            samples: self.samples,
        }
    }
    
    /// Reset the RTT estimator
    pub fn reset(&mut self) {
        self.rtt = 0.0;
        self.rtt_var = 0.0;
        self.history.clear();
        self.min_rtt = f64::MAX;
        self.max_rtt = 0.0;
        self.samples = 0;
        self.last_measurement = None;
        self.sr_timestamps.clear();
    }
}

/// RTT statistics
#[derive(Debug, Clone)]
pub struct RttStats {
    /// Current RTT estimate in milliseconds
    pub rtt_ms: f64,
    
    /// RTT variance in milliseconds
    pub rtt_var_ms: f64,
    
    /// Minimum RTT seen in milliseconds
    pub min_rtt_ms: f64,
    
    /// Maximum RTT seen in milliseconds
    pub max_rtt_ms: f64,
    
    /// Number of RTT samples
    pub samples: u64,
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_rtt_initial_state() {
        let estimator = RttEstimator::new();
        assert_eq!(estimator.get_rtt(), 0.0);
        assert_eq!(estimator.samples, 0);
        assert_eq!(estimator.history.len(), 0);
    }
    
    #[test]
    fn test_rtt_calculation() {
        let mut estimator = RttEstimator::new();
        
        // Record an SR sent with NTP timestamp
        // The NTP timestamp has seconds in the upper 32 bits, which we'll set to 0xabcd0000
        // In the receiver report, last_sr would be the middle 16 bits, which is 0xabcd
        let ssrc = 0x12345678;
        let ntp = NtpTimestamp { seconds: 0xabcd0000, fraction: 0x12345678 };
        estimator.record_sr_sent(ssrc, ntp);
        
        // Wait a bit to simulate network delay
        std::thread::sleep(Duration::from_millis(50));
        
        // Process a receiver report
        // The last_sr field in a receiver report is the middle 16 bits of NTP timestamp seconds
        let last_sr = 0xabcd;
        let delay = (0.01 * 65536.0) as u32; // 10ms delay, in Q16 format
        
        let rtt = estimator.process_receiver_report(ssrc, last_sr, delay);
        
        // We should have a valid RTT measurement
        assert!(rtt.is_some());
        
        // RTT should be around 40ms (50ms round trip minus 10ms delay)
        let rtt_ms = rtt.unwrap() * 1000.0;
        assert!(rtt_ms > 30.0 && rtt_ms < 100.0);
        
        // Check stats
        let stats = estimator.get_stats();
        assert_eq!(stats.samples, 1);
        assert!(stats.rtt_ms > 0.0);
        assert!(stats.min_rtt_ms > 0.0);
        assert!(stats.max_rtt_ms > 0.0);
    }
    
    #[test]
    fn test_rtt_tracking() {
        let mut estimator = RttEstimator::new();
        
        // Simulate several measurements
        for i in 0..5 {
            let ssrc = 0x12345678;
            // Create an NTP timestamp where the middle 16 bits of the seconds will be 0xabcd + i
            let ntp = NtpTimestamp { 
                seconds: ((0xabcd + i) << 16), 
                fraction: 0x12345678 
            };
            estimator.record_sr_sent(ssrc, ntp);
            
            // Simulate variable network delay
            let delay_ms = 50 + (i * 10); // 50, 60, 70, 80, 90 ms
            std::thread::sleep(Duration::from_millis(delay_ms.into()));
            
            // The last_sr field in a receiver report is the middle 16 bits of NTP timestamp seconds
            let last_sr = (0xabcd + i) as u32;
            let delay = (0.01 * 65536.0) as u32; // 10ms delay
            
            let rtt = estimator.process_receiver_report(ssrc, last_sr, delay);
            assert!(rtt.is_some(), "Failed to calculate RTT for sample {}", i);
        }
        
        // We should have 5 samples
        assert_eq!(estimator.samples, 5);
        
        // Check stats
        let stats = estimator.get_stats();
        assert!(stats.max_rtt_ms > stats.min_rtt_ms);
    }
} 