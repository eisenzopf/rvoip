//! Media Synchronization module
//!
//! This module provides utilities for synchronizing multiple RTP streams
//! (such as audio and video) that may have different clock rates.
//!
//! It handles:
//! - Mapping between RTP timestamps and NTP/wall clock time
//! - Synchronizing streams with different clock rates
//! - Detecting and compensating for clock drift

use std::collections::HashMap;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};
use std::sync::{Arc, Mutex};

use crate::RtpSsrc;
use crate::packet::rtcp::NtpTimestamp;

/// Media synchronization context for a set of related RTP streams
///
/// This structure manages the mapping between RTP and NTP timestamps
/// for multiple streams and provides utilities for synchronizing them.
pub struct MediaSync {
    /// Maps SSRCs to their synchronization state
    streams: HashMap<RtpSsrc, StreamSyncData>,
    
    /// Reference SSRC (typically audio) for synchronization
    reference_ssrc: Option<RtpSsrc>,
    
    /// When the synchronization context was created
    created_at: Instant,
}

/// Synchronization data for a single RTP stream
pub struct StreamSyncData {
    /// SSRC of the stream
    pub ssrc: RtpSsrc,
    
    /// Clock rate of the stream (samples per second)
    pub clock_rate: u32,
    
    /// Last NTP timestamp from RTCP SR
    pub last_ntp: Option<NtpTimestamp>,
    
    /// RTP timestamp corresponding to the last NTP timestamp
    pub last_rtp: Option<u32>,
    
    /// Timestamp when last update was received
    pub last_update: Option<Instant>,
    
    /// Measured clock drift in parts-per-million (positive means faster than reference)
    pub clock_drift_ppm: f64,
    
    /// Running average of drift measurements
    pub drift_history: Vec<f64>,
}

impl MediaSync {
    /// Create a new media synchronization context
    pub fn new() -> Self {
        Self {
            streams: HashMap::new(),
            reference_ssrc: None,
            created_at: Instant::now(),
        }
    }
    
    /// Register a stream for synchronization
    pub fn register_stream(&mut self, ssrc: RtpSsrc, clock_rate: u32) {
        // Create sync data for the stream
        let sync_data = StreamSyncData {
            ssrc,
            clock_rate,
            last_ntp: None,
            last_rtp: None,
            last_update: None,
            clock_drift_ppm: 0.0,
            drift_history: Vec::new(),
        };
        
        // Add to streams map
        self.streams.insert(ssrc, sync_data);
        
        // If this is the first stream, use it as the reference
        if self.reference_ssrc.is_none() {
            self.reference_ssrc = Some(ssrc);
        }
    }
    
    /// Get a reference to the streams map
    pub fn get_streams(&self) -> &HashMap<RtpSsrc, StreamSyncData> {
        &self.streams
    }
    
    /// Set the reference stream for synchronization
    ///
    /// The reference stream is typically an audio stream, as audio
    /// sync issues are more noticeable than video sync issues.
    pub fn set_reference_stream(&mut self, ssrc: RtpSsrc) {
        if self.streams.contains_key(&ssrc) {
            self.reference_ssrc = Some(ssrc);
        }
    }
    
    /// Update synchronization data for a stream from an RTCP Sender Report
    pub fn update_from_sr(&mut self, ssrc: RtpSsrc, ntp: NtpTimestamp, rtp: u32) {
        // Get the current time
        let now = Instant::now();
        
        // Check if we know this stream
        if let Some(stream) = self.streams.get_mut(&ssrc) {
            // Store previous values for drift calculation
            let prev_ntp = stream.last_ntp;
            let prev_rtp = stream.last_rtp;
            let prev_update = stream.last_update;
            
            // Update with new values
            stream.last_ntp = Some(ntp);
            stream.last_rtp = Some(rtp);
            stream.last_update = Some(now);
            
            // Calculate drift if we have previous values
            if let (Some(p_ntp), Some(p_rtp), Some(p_update)) = (prev_ntp, prev_rtp, prev_update) {
                // Time elapsed in sender's clock
                let ntp_elapsed_seconds = ntp.to_u64().wrapping_sub(p_ntp.to_u64()) as f64 / 2.0_f64.powi(32);
                
                // Time elapsed in RTP clock cycles
                let rtp_elapsed_cycles = rtp.wrapping_sub(p_rtp) as f64 / stream.clock_rate as f64;
                
                // Time elapsed on our clock
                let our_elapsed = now.duration_since(p_update).as_secs_f64();
                
                // Calculate drift: difference between NTP and RTP elapsed times
                let raw_drift = (ntp_elapsed_seconds - rtp_elapsed_cycles) / ntp_elapsed_seconds;
                
                // Convert to PPM (parts per million)
                let drift_ppm = raw_drift * 1_000_000.0;
                
                // Update the stream's drift tracking
                stream.drift_history.push(drift_ppm);
                
                // Keep only recent history (last 10 measurements)
                if stream.drift_history.len() > 10 {
                    stream.drift_history.remove(0);
                }
                
                // Calculate average drift
                let avg_drift = stream.drift_history.iter().sum::<f64>() / stream.drift_history.len() as f64;
                stream.clock_drift_ppm = avg_drift;
            }
        }
    }
    
    /// Convert an RTP timestamp from one stream to the equivalent timestamp in another stream
    ///
    /// This allows synchronizing media from different streams that use different clock rates.
    pub fn convert_timestamp(&self, from_ssrc: RtpSsrc, to_ssrc: RtpSsrc, rtp_ts: u32) -> Option<u32> {
        // Get sync data for both streams
        let from_stream = self.streams.get(&from_ssrc)?;
        let to_stream = self.streams.get(&to_ssrc)?;
        
        // Check if we have the necessary mapping data
        if from_stream.last_ntp.is_none() || from_stream.last_rtp.is_none() ||
           to_stream.last_ntp.is_none() || to_stream.last_rtp.is_none() {
            return None;
        }
        
        // Unwrap values (safe due to checks above)
        let from_ntp = from_stream.last_ntp.unwrap();
        let from_rtp_ref = from_stream.last_rtp.unwrap();
        let to_ntp = to_stream.last_ntp.unwrap();
        let to_rtp_ref = to_stream.last_rtp.unwrap();
        
        // Calculate time difference in source RTP clock ticks
        let rtp_diff = rtp_ts.wrapping_sub(from_rtp_ref) as i64;
        
        // Convert to seconds
        let seconds_diff = rtp_diff as f64 / from_stream.clock_rate as f64;
        
        // Apply drift correction if needed
        let corrected_seconds = seconds_diff * (1.0 + from_stream.clock_drift_ppm / 1_000_000.0);
        
        // Convert to destination clock rate
        let to_rtp_diff = (corrected_seconds * to_stream.clock_rate as f64) as i64;
        
        // Calculate target RTP timestamp
        Some(to_rtp_ref.wrapping_add(to_rtp_diff as u32))
    }
    
    /// Convert an RTP timestamp to an NTP timestamp
    pub fn rtp_to_ntp(&self, ssrc: RtpSsrc, rtp_ts: u32) -> Option<NtpTimestamp> {
        // Get sync data for the stream
        let stream = self.streams.get(&ssrc)?;
        
        // Check if we have the necessary mapping data
        if stream.last_ntp.is_none() || stream.last_rtp.is_none() {
            return None;
        }
        
        // Unwrap values (safe due to checks above)
        let ntp_ref = stream.last_ntp.unwrap();
        let rtp_ref = stream.last_rtp.unwrap();
        
        // Calculate time difference in RTP clock ticks
        let rtp_diff = rtp_ts.wrapping_sub(rtp_ref) as i64;
        
        // Convert to seconds
        let seconds_diff = rtp_diff as f64 / stream.clock_rate as f64;
        
        // Apply drift correction if needed
        let corrected_seconds = seconds_diff * (1.0 + stream.clock_drift_ppm / 1_000_000.0);
        
        // Convert to NTP format
        let ntp_seconds_diff = corrected_seconds.trunc() as u64;
        let ntp_fraction_diff = ((corrected_seconds.fract() * 2.0_f64.powi(32)) as u64) & 0xFFFF_FFFF;
        
        // Calculate target NTP timestamp (64-bit)
        let ntp_ref_value = ntp_ref.to_u64();
        let target_ntp_value = ntp_ref_value.wrapping_add(
            (ntp_seconds_diff << 32) | ntp_fraction_diff
        );
        
        // Convert back to NtpTimestamp
        Some(NtpTimestamp::from_u64(target_ntp_value))
    }
    
    /// Convert an NTP timestamp to an RTP timestamp for a specific stream
    pub fn ntp_to_rtp(&self, ssrc: RtpSsrc, ntp: NtpTimestamp) -> Option<u32> {
        // Get sync data for the stream
        let stream = self.streams.get(&ssrc)?;
        
        // Check if we have the necessary mapping data
        if stream.last_ntp.is_none() || stream.last_rtp.is_none() {
            return None;
        }
        
        // Unwrap values (safe due to checks above)
        let ntp_ref = stream.last_ntp.unwrap();
        let rtp_ref = stream.last_rtp.unwrap();
        
        // Calculate time difference in seconds
        let ntp_ref_value = ntp_ref.to_u64();
        let ntp_value = ntp.to_u64();
        let seconds_diff = if ntp_value >= ntp_ref_value {
            let diff = ntp_value - ntp_ref_value;
            (diff >> 32) as f64 + ((diff & 0xFFFF_FFFF) as f64 / 2.0_f64.powi(32))
        } else {
            let diff = ntp_ref_value - ntp_value;
            -((diff >> 32) as f64 + ((diff & 0xFFFF_FFFF) as f64 / 2.0_f64.powi(32)))
        };
        
        // Apply drift correction if needed
        let corrected_seconds = seconds_diff / (1.0 + stream.clock_drift_ppm / 1_000_000.0);
        
        // Convert to RTP clock ticks
        let rtp_diff = (corrected_seconds * stream.clock_rate as f64) as i64;
        
        // Calculate target RTP timestamp
        Some(rtp_ref.wrapping_add(rtp_diff as u32))
    }
    
    /// Get clock drift for a stream in parts per million
    pub fn get_clock_drift_ppm(&self, ssrc: RtpSsrc) -> Option<f64> {
        self.streams.get(&ssrc).map(|s| s.clock_drift_ppm)
    }
    
    /// Check if two streams are sufficiently synchronized
    ///
    /// Returns `true` if the streams have valid synchronization info
    /// and their clocks are aligned within the specified tolerance.
    pub fn are_synchronized(&self, ssrc1: RtpSsrc, ssrc2: RtpSsrc, tolerance_ms: f64) -> bool {
        // Get sync data for both streams
        let stream1 = match self.streams.get(&ssrc1) {
            Some(s) => s,
            None => return false,
        };
        
        let stream2 = match self.streams.get(&ssrc2) {
            Some(s) => s,
            None => return false,
        };
        
        // Check if both streams have sync data
        if stream1.last_ntp.is_none() || stream1.last_rtp.is_none() ||
           stream2.last_ntp.is_none() || stream2.last_rtp.is_none() {
            return false;
        }
        
        // Both streams have synchronization points, so they can be synchronized
        // Now check if drift is within acceptable limits
        
        // Convert tolerance to seconds
        let tolerance_sec = tolerance_ms / 1000.0;
        
        // Calculate maximum allowable drift based on session duration
        let session_duration = Instant::now().duration_since(self.created_at).as_secs_f64();
        let max_drift_ppm = (tolerance_sec / session_duration) * 1_000_000.0;
        
        // Calculate drift between streams
        let drift_diff = (stream1.clock_drift_ppm - stream2.clock_drift_ppm).abs();
        
        // Streams are synchronized if drift difference is within limits
        drift_diff <= max_drift_ppm
    }
}

impl Default for MediaSync {
    fn default() -> Self {
        Self::new()
    }
}

pub mod clock;
pub mod mapping; 