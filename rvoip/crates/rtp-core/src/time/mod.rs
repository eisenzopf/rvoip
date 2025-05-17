//! Time and clock utilities for RTP
//!
//! This module provides utilities for handling RTP timestamps, NTP timestamps,
//! and other time-related functions needed for RTP and RTCP.

use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// Convert RTP timestamp to duration at a given clock rate
pub fn rtp_timestamp_to_duration(timestamp: u32, clock_rate: u32) -> Duration {
    if clock_rate == 0 {
        return Duration::from_secs(0);
    }
    
    let seconds = timestamp / clock_rate;
    let remainder = timestamp % clock_rate;
    let nanos = ((remainder as u64) * 1_000_000_000) / (clock_rate as u64);
    
    Duration::new(seconds as u64, nanos as u32)
}

/// Convert duration to RTP timestamp at a given clock rate
pub fn duration_to_rtp_timestamp(duration: Duration, clock_rate: u32) -> u32 {
    let seconds = duration.as_secs();
    let nanos = duration.subsec_nanos();
    
    let timestamp_seconds = seconds * (clock_rate as u64);
    let timestamp_fraction = ((nanos as u64) * (clock_rate as u64)) / 1_000_000_000;
    
    (timestamp_seconds + timestamp_fraction) as u32
}

/// Get current RTP timestamp at a given clock rate
pub fn current_rtp_timestamp(clock_rate: u32) -> u32 {
    let now = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_else(|_| Duration::from_secs(0));
    
    duration_to_rtp_timestamp(now, clock_rate)
}

/// Calculate difference between two RTP timestamps, handling wraparound
pub fn rtp_timestamp_diff(a: u32, b: u32) -> u32 {
    // Handle wraparound
    if a > b && a - b > 0x80000000 {
        // b has wrapped around past a
        return b + (0xFFFFFFFF - a) + 1;
    } else if b > a && b - a > 0x80000000 {
        // a has wrapped around past b
        return a + (0xFFFFFFFF - b) + 1;
    } else if a > b {
        return a - b;
    } else {
        return b - a;
    }
}

/// Typical clock rates for common audio codecs
pub mod clock_rates {
    /// G.711, G.726, G.729 (8kHz)
    pub const AUDIO_8KHZ: u32 = 8000;
    
    /// G.722 (16kHz)
    pub const AUDIO_16KHZ: u32 = 16000;
    
    /// Opus, AAC (48kHz)
    pub const AUDIO_48KHZ: u32 = 48000;
    
    /// Typical video clock rate (90kHz)
    pub const VIDEO_90KHZ: u32 = 90000;
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_timestamp_conversion() {
        // Test with 8kHz clock rate
        let duration = Duration::from_millis(125);
        let timestamp = duration_to_rtp_timestamp(duration, 8000);
        assert_eq!(timestamp, 1000); // 125ms = 1000 samples at 8kHz
        
        let converted_duration = rtp_timestamp_to_duration(timestamp, 8000);
        assert_eq!(converted_duration.as_millis(), 125);
        
        // Test with 48kHz clock rate
        let duration = Duration::from_secs(1);
        let timestamp = duration_to_rtp_timestamp(duration, 48000);
        assert_eq!(timestamp, 48000); // 1s = 48000 samples at 48kHz
        
        let converted_duration = rtp_timestamp_to_duration(timestamp, 48000);
        assert_eq!(converted_duration.as_secs(), 1);
    }
    
    #[test]
    fn test_timestamp_diff() {
        // Normal cases
        assert_eq!(rtp_timestamp_diff(1000, 2000), 1000);
        assert_eq!(rtp_timestamp_diff(2000, 1000), 1000);
        
        // Wraparound cases
        assert_eq!(rtp_timestamp_diff(0xFFFFFFFF, 10), 11);
        assert_eq!(rtp_timestamp_diff(10, 0xFFFFFFFF), 11);
        
        // Large differences that aren't wraparounds
        assert_eq!(rtp_timestamp_diff(1, 0x70000000), 0x70000000 - 1);
        assert_eq!(rtp_timestamp_diff(0x70000000, 1), 0x70000000 - 1);
    }
} 