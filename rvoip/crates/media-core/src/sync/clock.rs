use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use tracing::{debug, trace, warn};

/// Media timestamp in RTP time
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct MediaTimestamp {
    /// RTP timestamp value
    pub rtp_timestamp: u32,
    /// Clock rate in Hz
    pub clock_rate: u32,
}

impl MediaTimestamp {
    /// Create a new media timestamp
    pub fn new(rtp_timestamp: u32, clock_rate: u32) -> Self {
        Self {
            rtp_timestamp,
            clock_rate,
        }
    }
    
    /// Convert RTP timestamp to duration since epoch
    pub fn as_duration(&self) -> Duration {
        Duration::from_secs_f64(self.rtp_timestamp as f64 / self.clock_rate as f64)
    }
    
    /// Calculate the difference between two timestamps
    pub fn diff(&self, other: &Self) -> Duration {
        if self.clock_rate != other.clock_rate {
            warn!("Comparing timestamps with different clock rates: {} vs {}", 
                  self.clock_rate, other.clock_rate);
        }
        
        // Calculate the difference
        let ts_diff = self.rtp_timestamp.wrapping_sub(other.rtp_timestamp);
        Duration::from_secs_f64(ts_diff as f64 / self.clock_rate as f64)
    }
    
    /// Advanced the timestamp by a duration
    pub fn advance(&self, duration: Duration) -> Self {
        let duration_secs = duration.as_secs_f64();
        let timestamp_offset = (duration_secs * self.clock_rate as f64) as u32;
        
        Self {
            rtp_timestamp: self.rtp_timestamp.wrapping_add(timestamp_offset),
            clock_rate: self.clock_rate,
        }
    }
    
    /// Convert to a different clock rate
    pub fn with_clock_rate(&self, new_clock_rate: u32) -> Self {
        if self.clock_rate == new_clock_rate {
            return *self;
        }
        
        let timestamp_seconds = self.rtp_timestamp as f64 / self.clock_rate as f64;
        let new_timestamp = (timestamp_seconds * new_clock_rate as f64) as u32;
        
        Self {
            rtp_timestamp: new_timestamp,
            clock_rate: new_clock_rate,
        }
    }
}

/// Clock source for a media clock
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClockSource {
    /// System clock
    System,
    /// Network Time Protocol (NTP)
    Ntp,
    /// RTP timestamps
    Rtp,
    /// External clock source
    External,
}

/// Media clock error types
#[derive(Debug, thiserror::Error)]
pub enum MediaClockError {
    /// The clock is not synchronized yet
    #[error("Clock not synchronized")]
    NotSynchronized,
    
    /// Timestamp source is invalid
    #[error("Invalid timestamp source")]
    InvalidSource,
    
    /// Clock drift is too high
    #[error("Clock drift exceeds threshold: {0} ms")]
    ExcessiveDrift(i64),
}

/// Media clock for managing media timing
pub struct MediaClock {
    /// Clock source
    source: ClockSource,
    /// Clock rate in Hz
    clock_rate: u32,
    /// Base RTP timestamp
    base_rtp: u32,
    /// Base system time
    base_time: Instant,
    /// Drift correction in ms (positive means system clock is ahead)
    drift_correction_ms: i64,
    /// Maximum allowed drift in ms
    max_drift_ms: i64,
    /// Whether the clock is synchronized
    synchronized: bool,
    /// Drift history for monitoring
    drift_history: Vec<(Instant, i64)>,
}

impl MediaClock {
    /// Create a new media clock
    pub fn new(clock_rate: u32, source: ClockSource) -> Self {
        Self {
            source,
            clock_rate,
            base_rtp: 0,
            base_time: Instant::now(),
            drift_correction_ms: 0,
            max_drift_ms: 100, // Default to 100ms max drift
            synchronized: false,
            drift_history: Vec::with_capacity(100),
        }
    }
    
    /// Synchronize the clock with an RTP timestamp
    pub fn synchronize(&mut self, rtp_timestamp: u32) {
        self.base_rtp = rtp_timestamp;
        self.base_time = Instant::now();
        self.synchronized = true;
        self.drift_correction_ms = 0;
        self.drift_history.clear();
        
        debug!("Media clock synchronized: rtp={}, rate={}Hz", 
               rtp_timestamp, self.clock_rate);
    }
    
    /// Update the clock with a new RTP timestamp
    pub fn update(&mut self, rtp_timestamp: u32) -> Result<(), MediaClockError> {
        if !self.synchronized {
            return Err(MediaClockError::NotSynchronized);
        }
        
        let now = Instant::now();
        let elapsed = now.duration_since(self.base_time);
        
        // Calculate expected RTP timestamp
        let expected_offset = (elapsed.as_secs_f64() * self.clock_rate as f64) as u32;
        let expected_rtp = self.base_rtp.wrapping_add(expected_offset);
        
        // Calculate drift in timestamp units
        let drift_ts = rtp_timestamp.wrapping_sub(expected_rtp) as i32;
        
        // Convert to milliseconds
        let drift_ms = drift_ts as i64 * 1000 / self.clock_rate as i64;
        
        // Update drift history
        self.drift_history.push((now, drift_ms));
        if self.drift_history.len() > 100 {
            self.drift_history.remove(0);
        }
        
        // Check if drift exceeds threshold
        if drift_ms.abs() > self.max_drift_ms {
            warn!("Media clock drift exceeds threshold: {}ms", drift_ms);
            return Err(MediaClockError::ExcessiveDrift(drift_ms));
        }
        
        // Apply gradual drift correction
        let correction_factor = 0.1; // Adjust 10% of the drift at a time
        let correction = (drift_ms as f64 * correction_factor) as i64;
        
        self.drift_correction_ms += correction;
        
        trace!("Media clock updated: rtp={}, drift={}ms, correction={}ms",
               rtp_timestamp, drift_ms, self.drift_correction_ms);
        
        Ok(())
    }
    
    /// Get the current RTP timestamp
    pub fn current_rtp_timestamp(&self) -> Result<u32, MediaClockError> {
        if !self.synchronized {
            return Err(MediaClockError::NotSynchronized);
        }
        
        let now = Instant::now();
        let mut elapsed = now.duration_since(self.base_time);
        
        // Apply drift correction
        let drift_correction = Duration::from_millis(self.drift_correction_ms.unsigned_abs());
        if self.drift_correction_ms > 0 {
            // System clock is ahead, subtract the correction
            elapsed = elapsed.saturating_sub(drift_correction);
        } else if self.drift_correction_ms < 0 {
            // System clock is behind, add the correction
            elapsed += drift_correction;
        }
        
        // Calculate RTP timestamp
        let rtp_offset = (elapsed.as_secs_f64() * self.clock_rate as f64) as u32;
        let current_rtp = self.base_rtp.wrapping_add(rtp_offset);
        
        Ok(current_rtp)
    }
    
    /// Get a media timestamp
    pub fn current_timestamp(&self) -> Result<MediaTimestamp, MediaClockError> {
        Ok(MediaTimestamp::new(
            self.current_rtp_timestamp()?,
            self.clock_rate,
        ))
    }
    
    /// Convert RTP timestamp to system time
    pub fn rtp_to_time(&self, rtp_timestamp: u32) -> Result<Instant, MediaClockError> {
        if !self.synchronized {
            return Err(MediaClockError::NotSynchronized);
        }
        
        // Calculate difference in RTP timestamps
        let rtp_diff = rtp_timestamp.wrapping_sub(self.base_rtp);
        
        // Convert to duration
        let time_diff = Duration::from_secs_f64(rtp_diff as f64 / self.clock_rate as f64);
        
        // Apply drift correction
        let drift_correction = Duration::from_millis(self.drift_correction_ms.unsigned_abs());
        let time = if self.drift_correction_ms > 0 {
            // System clock is ahead, add the correction to result
            self.base_time + time_diff + drift_correction
        } else if self.drift_correction_ms < 0 {
            // System clock is behind, subtract the correction from result
            self.base_time + time_diff - drift_correction
        } else {
            self.base_time + time_diff
        };
        
        Ok(time)
    }
    
    /// Convert system time to RTP timestamp
    pub fn time_to_rtp(&self, time: Instant) -> Result<u32, MediaClockError> {
        if !self.synchronized {
            return Err(MediaClockError::NotSynchronized);
        }
        
        // Calculate time difference
        let mut time_diff = time.duration_since(self.base_time);
        
        // Apply drift correction
        let drift_correction = Duration::from_millis(self.drift_correction_ms.unsigned_abs());
        if self.drift_correction_ms > 0 {
            // System clock is ahead, add the correction
            time_diff += drift_correction;
        } else if self.drift_correction_ms < 0 {
            // System clock is behind, subtract the correction
            time_diff = time_diff.saturating_sub(drift_correction);
        }
        
        // Convert to RTP timestamp
        let rtp_diff = (time_diff.as_secs_f64() * self.clock_rate as f64) as u32;
        let rtp_timestamp = self.base_rtp.wrapping_add(rtp_diff);
        
        Ok(rtp_timestamp)
    }
    
    /// Set the maximum allowed drift
    pub fn set_max_drift(&mut self, max_drift_ms: i64) {
        self.max_drift_ms = max_drift_ms;
    }
    
    /// Get the current drift in milliseconds
    pub fn current_drift_ms(&self) -> i64 {
        self.drift_correction_ms
    }
    
    /// Reset the clock
    pub fn reset(&mut self) {
        self.synchronized = false;
        self.base_time = Instant::now();
        self.base_rtp = 0;
        self.drift_correction_ms = 0;
        self.drift_history.clear();
        
        debug!("Media clock reset");
    }
    
    /// Check if the clock is synchronized
    pub fn is_synchronized(&self) -> bool {
        self.synchronized
    }
    
    /// Get the clock rate
    pub fn clock_rate(&self) -> u32 {
        self.clock_rate
    }
    
    /// Get the clock source
    pub fn source(&self) -> ClockSource {
        self.source
    }
    
    /// Get the drift history
    pub fn drift_history(&self) -> &[(Instant, i64)] {
        &self.drift_history
    }
    
    /// Create a shared media clock
    pub fn shared(clock_rate: u32, source: ClockSource) -> Arc<Mutex<Self>> {
        Arc::new(Mutex::new(Self::new(clock_rate, source)))
    }
} 