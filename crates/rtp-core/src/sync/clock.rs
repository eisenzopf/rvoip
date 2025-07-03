//! Media clock utility module
//!
//! This module provides utilities for working with different clock domains
//! and handling clock rate conversions for media synchronization.

use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use crate::packet::rtcp::NtpTimestamp;
use crate::RtpTimestamp;

/// Media clock source
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockSource {
    /// System wall clock
    System,
    
    /// NTP clock from RTCP
    Ntp,
    
    /// RTP media clock with a specific rate
    Rtp(u32), // Clock rate in Hz
}

/// A media clock that can convert between timestamp domains
#[derive(Debug, Clone)]
pub struct MediaClock {
    /// Clock rate in samples per second
    clock_rate: u32,
    
    /// Reference RTP timestamp
    ref_rtp_ts: RtpTimestamp,
    
    /// Reference NTP timestamp
    ref_ntp_ts: NtpTimestamp,
    
    /// Reference system time
    ref_system_time: Instant,
    
    /// Clock drift in parts-per-million (positive means faster than system clock)
    drift_ppm: f64,
}

impl MediaClock {
    /// Create a new media clock with the given rate and reference timestamps
    pub fn new(
        clock_rate: u32, 
        ref_rtp_ts: RtpTimestamp, 
        ref_ntp_ts: NtpTimestamp
    ) -> Self {
        Self {
            clock_rate,
            ref_rtp_ts,
            ref_ntp_ts,
            ref_system_time: Instant::now(),
            drift_ppm: 0.0,
        }
    }
    
    /// Create a media clock from the current system time
    pub fn now(clock_rate: u32, initial_rtp_ts: RtpTimestamp) -> Self {
        Self::new(clock_rate, initial_rtp_ts, NtpTimestamp::now())
    }
    
    /// Get the clock rate
    pub fn clock_rate(&self) -> u32 {
        self.clock_rate
    }
    
    /// Set the estimated clock drift
    pub fn set_drift(&mut self, ppm: f64) {
        self.drift_ppm = ppm;
    }
    
    /// Get the estimated clock drift
    pub fn drift(&self) -> f64 {
        self.drift_ppm
    }
    
    /// Update the reference timestamps
    pub fn update_reference(&mut self, rtp_ts: RtpTimestamp, ntp_ts: NtpTimestamp) {
        self.ref_rtp_ts = rtp_ts;
        self.ref_ntp_ts = ntp_ts;
        self.ref_system_time = Instant::now();
    }
    
    /// Convert an RTP timestamp to a system time
    pub fn rtp_to_system_time(&self, rtp_ts: RtpTimestamp) -> Instant {
        // Calculate time difference in RTP clock ticks
        let ticks_diff = rtp_ts.wrapping_sub(self.ref_rtp_ts) as i64;
        
        // Convert to seconds, adjusting for clock drift
        let seconds_diff = (ticks_diff as f64 / self.clock_rate as f64) / 
                           (1.0 + self.drift_ppm / 1_000_000.0);
        
        // Convert to duration and add to reference time
        if seconds_diff >= 0.0 {
            self.ref_system_time + Duration::from_secs_f64(seconds_diff)
        } else {
            // Handle negative time difference
            let abs_diff = -seconds_diff;
            self.ref_system_time - Duration::from_secs_f64(abs_diff)
        }
    }
    
    /// Convert a system time to an RTP timestamp
    pub fn system_time_to_rtp(&self, time: Instant) -> RtpTimestamp {
        // Calculate time difference in seconds
        let seconds_diff = if time >= self.ref_system_time {
            time.duration_since(self.ref_system_time).as_secs_f64()
        } else {
            -self.ref_system_time.duration_since(time).as_secs_f64()
        };
        
        // Apply clock drift
        let corrected_seconds = seconds_diff * (1.0 + self.drift_ppm / 1_000_000.0);
        
        // Convert to RTP clock ticks
        let ticks_diff = (corrected_seconds * self.clock_rate as f64) as i64;
        
        // Add to reference timestamp
        self.ref_rtp_ts.wrapping_add(ticks_diff as u32)
    }
    
    /// Convert an RTP timestamp to an NTP timestamp
    pub fn rtp_to_ntp(&self, rtp_ts: RtpTimestamp) -> NtpTimestamp {
        // Calculate time difference in RTP clock ticks
        let ticks_diff = rtp_ts.wrapping_sub(self.ref_rtp_ts) as i64;
        
        // Convert to seconds
        let seconds_diff = ticks_diff as f64 / self.clock_rate as f64;
        
        // Apply drift correction if needed
        let corrected_seconds = seconds_diff / (1.0 + self.drift_ppm / 1_000_000.0);
        
        // Calculate seconds and fraction parts
        let whole_seconds = corrected_seconds.trunc() as u64;
        let fraction = (corrected_seconds.fract() * 2.0_f64.powi(32)) as u64;
        
        // Reference NTP value
        let ntp_ref_value = self.ref_ntp_ts.to_u64();
        
        // Calculate final NTP value
        let ntp_value = ntp_ref_value.wrapping_add((whole_seconds << 32) | (fraction & 0xFFFF_FFFF));
        
        // Convert back to NTP timestamp
        NtpTimestamp::from_u64(ntp_value)
    }
    
    /// Convert an NTP timestamp to an RTP timestamp
    pub fn ntp_to_rtp(&self, ntp_ts: NtpTimestamp) -> RtpTimestamp {
        // Convert NTP timestamps to 64-bit values
        let ntp_ref_value = self.ref_ntp_ts.to_u64();
        let ntp_value = ntp_ts.to_u64();
        
        // Calculate time difference in seconds
        let seconds_diff = if ntp_value >= ntp_ref_value {
            // Positive difference
            let diff = ntp_value - ntp_ref_value;
            (diff >> 32) as f64 + ((diff & 0xFFFF_FFFF) as f64 / 2.0_f64.powi(32))
        } else {
            // Negative difference
            let diff = ntp_ref_value - ntp_value;
            -((diff >> 32) as f64 + ((diff & 0xFFFF_FFFF) as f64 / 2.0_f64.powi(32)))
        };
        
        // Apply drift correction
        let corrected_seconds = seconds_diff * (1.0 + self.drift_ppm / 1_000_000.0);
        
        // Convert to RTP clock ticks
        let ticks_diff = (corrected_seconds * self.clock_rate as f64) as i64;
        
        // Add to reference timestamp
        self.ref_rtp_ts.wrapping_add(ticks_diff as u32)
    }
    
    /// Convert a timestamp between different clock rates
    pub fn convert_clock_rate(&self, timestamp: RtpTimestamp, from_rate: u32, to_rate: u32) -> RtpTimestamp {
        if from_rate == to_rate {
            return timestamp;
        }
        
        // Convert to a time in seconds
        let seconds = timestamp as f64 / from_rate as f64;
        
        // Convert to the target clock rate
        let new_timestamp = (seconds * to_rate as f64) as u32;
        
        new_timestamp
    }
}

/// Calculate the synchronization offset between two streams
///
/// Returns the offset in milliseconds that should be applied to `stream2`
/// to synchronize it with `stream1`.
pub fn calculate_sync_offset(
    stream1_rtp: RtpTimestamp,
    stream1_ntp: NtpTimestamp,
    stream1_rate: u32,
    stream2_rtp: RtpTimestamp,
    stream2_ntp: NtpTimestamp,
    stream2_rate: u32,
) -> f64 {
    // Convert both RTP timestamps to NTP time
    let stream1_time = stream1_rtp as f64 / stream1_rate as f64;
    let stream2_time = stream2_rtp as f64 / stream2_rate as f64;
    
    // Calculate RTP time difference in seconds
    // (how far each RTP timestamp is from its NTP reference point)
    let stream1_ntp_val = stream1_ntp.to_u64();
    let stream2_ntp_val = stream2_ntp.to_u64();
    
    // Convert timestamps to a common time scale (seconds)
    let stream1_ntp_seconds = (stream1_ntp_val >> 32) as f64 + 
                             ((stream1_ntp_val & 0xFFFF_FFFF) as f64) / 2.0_f64.powi(32);
    let stream2_ntp_seconds = (stream2_ntp_val >> 32) as f64 + 
                             ((stream2_ntp_val & 0xFFFF_FFFF) as f64) / 2.0_f64.powi(32);
    
    // Calculate offset in seconds
    let offset_seconds = (stream2_ntp_seconds - stream1_ntp_seconds) + (stream1_time - stream2_time);
    
    // Convert to milliseconds
    offset_seconds * 1000.0
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_media_clock_conversion() {
        // Create a reference time
        let ref_ntp = NtpTimestamp::now();
        let ref_rtp = 48000;
        let clock_rate = 8000;
        
        let mut clock = MediaClock::new(clock_rate, ref_rtp, ref_ntp);
        
        // Test RTP to NTP conversion (should be close to reference)
        let ntp = clock.rtp_to_ntp(ref_rtp);
        assert_eq!(ntp.seconds, ref_ntp.seconds);
        assert_eq!(ntp.fraction, ref_ntp.fraction);
        
        // Test NTP to RTP conversion (should be close to reference)
        let rtp = clock.ntp_to_rtp(ref_ntp);
        assert_eq!(rtp, ref_rtp);
        
        // Test clock rate conversion
        let ts_8khz = 8000; // 1 second at 8kHz
        let ts_16khz = clock.convert_clock_rate(ts_8khz, 8000, 16000);
        assert_eq!(ts_16khz, 16000); // Should be 1 second at 16kHz
        
        let ts_48khz = clock.convert_clock_rate(ts_8khz, 8000, 48000);
        assert_eq!(ts_48khz, 48000); // Should be 1 second at 48kHz
    }
    
    #[test]
    fn test_sync_offset_calculation() {
        // Create two NTP timestamps 100ms apart
        let ntp1 = NtpTimestamp::now();
        let ntp1_val = ntp1.to_u64();
        let ntp2_val = ntp1_val + (100 * 2_u64.pow(32) / 1000); // 100ms later
        let ntp2 = NtpTimestamp::from_u64(ntp2_val);
        
        // First stream at 8kHz, timestamp represents 200ms
        let rtp1 = 1600; // 8000Hz * 0.2s = 1600 samples
        
        // Second stream at 48kHz, timestamp represents 500ms
        let rtp2 = 24000; // 48000Hz * 0.5s = 24000 samples
        
        // Calculate the synchronization offset
        let offset = calculate_sync_offset(
            rtp1, ntp1, 8000,
            rtp2, ntp2, 48000
        );
        
        // Expected offset: (ntp2 - ntp1) + (rtp1/8000 - rtp2/48000) in ms
        // = 100ms + (200ms - 500ms) = -200ms
        assert!((offset + 200.0).abs() < 1.0); // Allow small floating point error
    }
} 