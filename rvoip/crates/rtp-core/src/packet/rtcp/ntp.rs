use std::time::{Duration, SystemTime, UNIX_EPOCH};

/// NTP timestamp representation (64 bits)
/// As defined in RFC 3550
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct NtpTimestamp {
    /// Seconds since January 1, 1900
    pub seconds: u32,
    
    /// Fraction of a second
    pub fraction: u32,
}

impl NtpTimestamp {
    /// Create a new NTP timestamp from the current system time
    pub fn now() -> Self {
        // Get current time since UNIX epoch
        let now = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_else(|_| Duration::from_secs(0));
        
        // Convert to NTP timestamp (seconds since January 1, 1900)
        // NTP epoch starts 70 years before UNIX epoch (2208988800 seconds)
        let ntp_seconds = now.as_secs() + 2208988800;
        
        // Convert nanoseconds to NTP fraction (2^32 / 10^9)
        let nanos = now.subsec_nanos();
        let ntp_fraction = (nanos as u64 * 0x100000000u64 / 1_000_000_000) as u32;
        
        Self {
            seconds: ntp_seconds as u32,
            fraction: ntp_fraction,
        }
    }
    
    /// Convert to a 64-bit representation
    pub fn to_u64(&self) -> u64 {
        (self.seconds as u64) << 32 | (self.fraction as u64)
    }
    
    /// Convert to a 32-bit representation for RTCP reports
    /// 
    /// Returns the middle 32 bits of the NTP timestamp, which is used in RTCP
    /// report blocks (last_sr field) for RTT calculations.
    /// This is defined in RFC 3550 Section 6.4.1.
    pub fn to_u32(&self) -> u32 {
        // Take the middle 16 bits of seconds and the most significant 16 bits of fraction
        ((self.seconds & 0x0000FFFF) << 16) | ((self.fraction & 0xFFFF0000) >> 16)
    }
    
    /// Convert from a 64-bit representation
    pub fn from_u64(value: u64) -> Self {
        Self {
            seconds: (value >> 32) as u32,
            fraction: value as u32,
        }
    }
    
    /// Convert to a Duration since UNIX epoch
    pub fn to_duration_since_unix_epoch(&self) -> Duration {
        // NTP epoch to UNIX epoch offset (70 years in seconds)
        const NTP_TO_UNIX_OFFSET: u64 = 2208988800;
        
        // Calculate seconds, handling underflow if the timestamp is before UNIX epoch
        let seconds = if self.seconds as u64 > NTP_TO_UNIX_OFFSET {
            self.seconds as u64 - NTP_TO_UNIX_OFFSET
        } else {
            0 // If timestamp predates UNIX epoch, return 0
        };
        
        // Convert fraction to nanoseconds (fraction * 10^9 / 2^32)
        let nanos = ((self.fraction as u64) * 1_000_000_000) >> 32;
        
        Duration::new(seconds, nanos as u32)
    }
    
    /// Create a new NTP timestamp from a Duration since UNIX epoch
    pub fn from_duration_since_unix_epoch(duration: Duration) -> Self {
        // NTP epoch to UNIX epoch offset (70 years in seconds)
        const UNIX_TO_NTP_OFFSET: u64 = 2208988800;
        
        // Convert seconds
        let seconds = duration.as_secs() + UNIX_TO_NTP_OFFSET;
        
        // Convert nanoseconds to fraction
        let nanos = duration.subsec_nanos();
        let fraction = ((nanos as u64 * 0x100000000u64) / 1_000_000_000) as u32;
        
        Self {
            seconds: seconds as u32,
            fraction,
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_ntp_timestamp_creation() {
        let timestamp = NtpTimestamp::now();
        
        // The timestamp should be roughly the current time
        // We just check it's in a reasonable range (after 2020)
        assert!(timestamp.seconds > 3786825600); // Jan 1, 2020 in NTP time
    }
    
    #[test]
    fn test_ntp_timestamp_conversion() {
        // Create a test timestamp
        let timestamp = NtpTimestamp {
            seconds: 3786825600, // Jan 1, 2020 in NTP time
            fraction: 0x80000000, // 0.5 seconds
        };
        
        // Convert to u64 and back
        let u64_value = timestamp.to_u64();
        let converted = NtpTimestamp::from_u64(u64_value);
        
        assert_eq!(converted.seconds, timestamp.seconds);
        assert_eq!(converted.fraction, timestamp.fraction);
    }
    
    #[test]
    fn test_ntp_timestamp_to_duration() {
        // Create a test timestamp for Jan 1, 2020, 00:00:00.5
        let timestamp = NtpTimestamp {
            seconds: 3786825600, // Jan 1, 2020 in NTP time
            fraction: 0x80000000, // 0.5 seconds
        };
        
        let duration = timestamp.to_duration_since_unix_epoch();
        
        // Expected: Jan 1, 2020 minus NTP epoch offset plus 0.5 seconds
        assert_eq!(duration.as_secs(), 1577836800); // Jan 1, 2020 in UNIX time
        assert!(duration.subsec_nanos() > 499000000 && duration.subsec_nanos() < 501000000);
    }
    
    #[test]
    fn test_ntp_timestamp_from_duration() {
        // Create a duration for Jan 1, 2020, 00:00:00.5
        let duration = Duration::new(1577836800, 500000000);
        
        let timestamp = NtpTimestamp::from_duration_since_unix_epoch(duration);
        
        // Expected: Jan 1, 2020 in NTP time plus 0.5 seconds
        assert_eq!(timestamp.seconds, 3786825600);
        
        // Fraction should be close to 0.5 (0x80000000)
        let expected = 0x80000000u32;
        let tolerance = 100; // Allow small rounding errors
        assert!(
            timestamp.fraction >= expected.saturating_sub(tolerance) && 
            timestamp.fraction <= expected.saturating_add(tolerance)
        );
    }
} 