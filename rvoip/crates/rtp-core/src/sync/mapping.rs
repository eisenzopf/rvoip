//! Timestamp mapping module
//!
//! This module provides mapping between different timestamp domains,
//! such as RTP timestamps from different streams, NTP timestamps,
//! and system time.

use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use std::sync::{Arc, Mutex};

use crate::RtpSsrc;
use crate::RtpTimestamp;
use crate::packet::rtcp::NtpTimestamp;
use super::clock::MediaClock;

/// Mapping between a source stream and a target stream
struct StreamMapping {
    /// Source stream SSRC
    source_ssrc: RtpSsrc,
    
    /// Target stream SSRC
    target_ssrc: RtpSsrc,
    
    /// Source stream clock rate
    source_rate: u32,
    
    /// Target stream clock rate
    target_rate: u32,
    
    /// Source stream reference RTP timestamp
    source_rtp_ref: RtpTimestamp,
    
    /// Target stream reference RTP timestamp
    target_rtp_ref: RtpTimestamp,
    
    /// Common reference NTP timestamp
    ntp_ref: NtpTimestamp,
    
    /// When this mapping was last updated
    last_update: Instant,
    
    /// Measured clock drift between source and target (parts per million)
    drift_ppm: f64,
}

impl StreamMapping {
    /// Create a new mapping between two streams
    pub fn new(
        source_ssrc: RtpSsrc,
        target_ssrc: RtpSsrc,
        source_rate: u32,
        target_rate: u32,
        source_rtp: RtpTimestamp,
        target_rtp: RtpTimestamp,
        ntp: NtpTimestamp,
    ) -> Self {
        Self {
            source_ssrc,
            target_ssrc,
            source_rate,
            target_rate,
            source_rtp_ref: source_rtp,
            target_rtp_ref: target_rtp,
            ntp_ref: ntp,
            last_update: Instant::now(),
            drift_ppm: 0.0,
        }
    }
    
    /// Map a source RTP timestamp to the equivalent in the target stream
    pub fn map_timestamp(&self, source_rtp: RtpTimestamp) -> RtpTimestamp {
        // Calculate time difference in source stream ticks
        let source_diff = source_rtp.wrapping_sub(self.source_rtp_ref) as i64;
        
        // Convert to seconds, using source clock rate
        let seconds = source_diff as f64 / self.source_rate as f64;
        
        // Apply drift correction
        let corrected_seconds = seconds * (1.0 + self.drift_ppm / 1_000_000.0);
        
        // Convert to target stream ticks
        let target_diff = (corrected_seconds * self.target_rate as f64) as i64;
        
        // Calculate target timestamp
        self.target_rtp_ref.wrapping_add(target_diff as u32)
    }
    
    /// Update the mapping with new reference timestamps
    pub fn update(
        &mut self, 
        source_rtp: RtpTimestamp, 
        target_rtp: RtpTimestamp, 
        ntp: NtpTimestamp
    ) -> f64 {
        // Calculate elapsed time between updates
        let now = Instant::now();
        let elapsed = now.duration_since(self.last_update).as_secs_f64();
        
        if elapsed > 0.0 {
            // Calculate drift between source and target
            let source_ticks = source_rtp.wrapping_sub(self.source_rtp_ref) as i64;
            let target_ticks = target_rtp.wrapping_sub(self.target_rtp_ref) as i64;
            
            let source_seconds = source_ticks as f64 / self.source_rate as f64;
            let target_seconds = target_ticks as f64 / self.target_rate as f64;
            
            // Calculate drift in PPM (parts per million)
            let drift = (target_seconds - source_seconds) / elapsed;
            self.drift_ppm = drift * 1_000_000.0;
        }
        
        // Update reference points
        self.source_rtp_ref = source_rtp;
        self.target_rtp_ref = target_rtp;
        self.ntp_ref = ntp;
        self.last_update = now;
        
        // Return drift in milliseconds per second
        self.drift_ppm / 1000.0
    }
}

/// Manages timestamp mappings between multiple streams
#[derive(Clone)]
pub struct TimestampMapper {
    /// Maps from (source_ssrc, target_ssrc) to StreamMapping
    mappings: Arc<Mutex<HashMap<(RtpSsrc, RtpSsrc), StreamMapping>>>,
    
    /// Maps from SSRC to its corresponding MediaClock
    clocks: Arc<Mutex<HashMap<RtpSsrc, MediaClock>>>,
}

impl TimestampMapper {
    /// Create a new timestamp mapper
    pub fn new() -> Self {
        Self {
            mappings: Arc::new(Mutex::new(HashMap::new())),
            clocks: Arc::new(Mutex::new(HashMap::new())),
        }
    }
    
    /// Register a new stream with its clock rate
    pub fn register_stream(&self, ssrc: RtpSsrc, clock_rate: u32, initial_rtp: RtpTimestamp) {
        if let Ok(mut clocks) = self.clocks.lock() {
            // Create a new media clock for this stream
            let clock = MediaClock::now(clock_rate, initial_rtp);
            clocks.insert(ssrc, clock);
        }
    }
    
    /// Update stream timing information from an RTCP sender report
    pub fn update_from_sr(&self, ssrc: RtpSsrc, ntp: NtpTimestamp, rtp: RtpTimestamp) {
        if let Ok(mut clocks) = self.clocks.lock() {
            // If we have a clock for this stream, update it
            if let Some(clock) = clocks.get_mut(&ssrc) {
                clock.update_reference(rtp, ntp);
            } else {
                // If not, create one if we know the clock rate
                // (fallback to common rates based on payload type)
                let clock_rate = 8000; // Default to 8kHz
                let clock = MediaClock::new(clock_rate, rtp, ntp);
                clocks.insert(ssrc, clock);
            }
        }
    }
    
    /// Create or update a mapping between two streams
    pub fn map_streams(
        &self, 
        source_ssrc: RtpSsrc,
        target_ssrc: RtpSsrc,
        source_rtp: RtpTimestamp,
        target_rtp: RtpTimestamp,
        ntp: NtpTimestamp,
    ) -> Option<f64> {
        // Get the clock rates
        let (source_rate, target_rate) = if let Ok(clocks) = self.clocks.lock() {
            let source_clock = clocks.get(&source_ssrc)?;
            let target_clock = clocks.get(&target_ssrc)?;
            (source_clock.clock_rate(), target_clock.clock_rate())
        } else {
            return None;
        };
        
        if let Ok(mut mappings) = self.mappings.lock() {
            let key = (source_ssrc, target_ssrc);
            
            // Check if mapping already exists
            if let Some(mapping) = mappings.get_mut(&key) {
                // Update existing mapping
                let drift = mapping.update(source_rtp, target_rtp, ntp);
                Some(drift)
            } else {
                // Create new mapping
                let mapping = StreamMapping::new(
                    source_ssrc, target_ssrc, source_rate, target_rate,
                    source_rtp, target_rtp, ntp
                );
                mappings.insert(key, mapping);
                Some(0.0) // No drift initially
            }
        } else {
            None
        }
    }
    
    /// Map a timestamp from one stream to another
    pub fn map_timestamp(
        &self,
        source_ssrc: RtpSsrc,
        target_ssrc: RtpSsrc,
        source_rtp: RtpTimestamp,
    ) -> Option<RtpTimestamp> {
        if let Ok(mappings) = self.mappings.lock() {
            let key = (source_ssrc, target_ssrc);
            
            if let Some(mapping) = mappings.get(&key) {
                // Direct mapping exists
                Some(mapping.map_timestamp(source_rtp))
            } else {
                // Try indirect mapping via NTP timestamps
                if let Ok(clocks) = self.clocks.lock() {
                    let source_clock = clocks.get(&source_ssrc)?;
                    let target_clock = clocks.get(&target_ssrc)?;
                    
                    // Convert source RTP to NTP
                    let ntp = source_clock.rtp_to_ntp(source_rtp);
                    
                    // Convert NTP to target RTP
                    Some(target_clock.ntp_to_rtp(ntp))
                } else {
                    None
                }
            }
        } else {
            None
        }
    }
    
    /// Get estimated clock drift between two streams in PPM
    pub fn get_drift(&self, source_ssrc: RtpSsrc, target_ssrc: RtpSsrc) -> Option<f64> {
        if let Ok(mappings) = self.mappings.lock() {
            let key = (source_ssrc, target_ssrc);
            
            mappings.get(&key).map(|mapping| mapping.drift_ppm)
        } else {
            None
        }
    }
    
    /// Convert an RTP timestamp to wall clock time
    pub fn rtp_to_wallclock(&self, ssrc: RtpSsrc, rtp: RtpTimestamp) -> Option<Instant> {
        if let Ok(clocks) = self.clocks.lock() {
            let clock = clocks.get(&ssrc)?;
            Some(clock.rtp_to_system_time(rtp))
        } else {
            None
        }
    }
    
    /// Convert wall clock time to an RTP timestamp
    pub fn wallclock_to_rtp(&self, ssrc: RtpSsrc, time: Instant) -> Option<RtpTimestamp> {
        if let Ok(clocks) = self.clocks.lock() {
            let clock = clocks.get(&ssrc)?;
            Some(clock.system_time_to_rtp(time))
        } else {
            None
        }
    }
    
    /// Get the synchronization offset between two streams in milliseconds
    ///
    /// Returns a positive value if target stream is ahead of source stream
    /// and should be delayed to achieve synchronization.
    pub fn get_sync_offset(&self, source_ssrc: RtpSsrc, target_ssrc: RtpSsrc) -> Option<f64> {
        // Try to get the direct mapping
        if let Ok(mappings) = self.mappings.lock() {
            let key = (source_ssrc, target_ssrc);
            
            if let Some(mapping) = mappings.get(&key) {
                // Calculate how far the streams have drifted
                let elapsed = Instant::now().duration_since(mapping.last_update).as_secs_f64();
                
                // Drift in ms per second * elapsed seconds = total drift in ms
                let drift_ms = (mapping.drift_ppm / 1000.0) * elapsed;
                
                return Some(drift_ms);
            }
        }
        
        // If no direct mapping, try to calculate via NTP timestamps
        if let Ok(clocks) = self.clocks.lock() {
            let source_clock = clocks.get(&source_ssrc)?;
            let target_clock = clocks.get(&target_ssrc)?;
            
            // Create a common reference point (now)
            let now = SystemTime::now().duration_since(UNIX_EPOCH).ok()?;
            let ntp = NtpTimestamp::from_duration_since_unix_epoch(now);
            
            // Convert to RTP timestamps in each stream's clock domain
            let source_rtp = source_clock.ntp_to_rtp(ntp);
            let target_rtp = target_clock.ntp_to_rtp(ntp);
            
            // Calculate playback points in seconds
            let source_seconds = source_rtp as f64 / source_clock.clock_rate() as f64;
            let target_seconds = target_rtp as f64 / target_clock.clock_rate() as f64;
            
            // Calculate offset in milliseconds
            let offset_ms = (target_seconds - source_seconds) * 1000.0;
            
            Some(offset_ms)
        } else {
            None
        }
    }
}

impl Default for TimestampMapper {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_stream_mapping() {
        // Create a mapping between two streams
        let source_ssrc = 0x1234;
        let target_ssrc = 0x5678;
        let source_rate = 8000;  // 8 kHz audio
        let target_rate = 90000; // 90 kHz video
        let source_rtp = 1600;   // 200ms of audio
        let target_rtp = 18000;  // 200ms of video
        let ntp = NtpTimestamp::now();
        
        let mut mapping = StreamMapping::new(
            source_ssrc, target_ssrc, source_rate, target_rate,
            source_rtp, target_rtp, ntp
        );
        
        // Test mapping 400ms (3200 samples) of audio to video
        let source_rtp_400ms = source_rtp.wrapping_add(3200);
        let target_rtp_400ms = mapping.map_timestamp(source_rtp_400ms);
        
        // Expected: target_rtp + (400ms - 200ms) * 90000 = target_rtp + 18000
        let expected = target_rtp.wrapping_add(18000);
        assert_eq!(target_rtp_400ms, expected);
    }
    
    #[test]
    fn test_timestamp_mapper() {
        let mapper = TimestampMapper::new();
        
        // Register two streams
        let audio_ssrc = 0x1234;
        let video_ssrc = 0x5678;
        let audio_rate = 8000;  // 8 kHz
        let video_rate = 90000; // 90 kHz
        
        mapper.register_stream(audio_ssrc, audio_rate, 800); // 100ms
        mapper.register_stream(video_ssrc, video_rate, 9000); // 100ms
        
        // Create mapping with both at 200ms
        let ntp = NtpTimestamp::now();
        let audio_rtp_200ms = 1600; // 200ms at 8kHz
        let video_rtp_200ms = 18000; // 200ms at 90kHz
        
        mapper.map_streams(audio_ssrc, video_ssrc, audio_rtp_200ms, video_rtp_200ms, ntp);
        
        // Map 400ms of audio to video
        let audio_rtp_400ms = 3200; // 400ms at 8kHz
        let video_rtp_400ms = mapper.map_timestamp(audio_ssrc, video_ssrc, audio_rtp_400ms);
        
        // Expected: video_rtp_200ms + (400ms - 200ms) * 90kHz = video_rtp_200ms + 18000
        assert_eq!(video_rtp_400ms, Some(video_rtp_200ms + 18000));
    }
} 