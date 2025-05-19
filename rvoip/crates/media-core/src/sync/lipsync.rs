use std::collections::VecDeque;
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use bytes::Bytes;
use tracing::{debug, trace, warn};

use crate::error::Result;
use crate::sync::clock::{MediaClock, MediaTimestamp};

/// Media synchronization point
#[derive(Debug, Clone)]
pub struct MediaSyncPoint {
    /// Media stream ID
    pub stream_id: String,
    /// Media type (audio/video)
    pub media_type: MediaType,
    /// RTP timestamp
    pub timestamp: MediaTimestamp,
    /// Capture time
    pub capture_time: Option<Instant>,
    /// Presentation time
    pub presentation_time: Option<Instant>,
}

/// Media type for synchronization
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum MediaType {
    /// Audio stream
    Audio,
    /// Video stream
    Video,
}

/// Lip sync configuration
#[derive(Debug, Clone)]
pub struct LipSyncConfig {
    /// Maximum audio buffer size in milliseconds
    pub max_audio_buffer_ms: u32,
    /// Maximum video buffer size in milliseconds
    pub max_video_buffer_ms: u32,
    /// Target audio-video offset in milliseconds (positive means audio is ahead)
    pub target_av_offset_ms: i32,
    /// Synchronization window in milliseconds
    pub sync_window_ms: u32,
    /// Whether to adapt to network conditions
    pub adaptive: bool,
}

impl Default for LipSyncConfig {
    fn default() -> Self {
        Self {
            max_audio_buffer_ms: 200,
            max_video_buffer_ms: 500,
            target_av_offset_ms: 0,  // Ideally perfectly synchronized
            sync_window_ms: 20,      // 20ms synchronization window
            adaptive: true,
        }
    }
}

/// Media buffer entry
#[derive(Debug, Clone)]
struct MediaEntry {
    /// Media data
    data: Bytes,
    /// RTP timestamp
    timestamp: MediaTimestamp,
    /// Capture time
    capture_time: Instant,
    /// Scheduled presentation time
    presentation_time: Option<Instant>,
}

/// Lip sync manager for audio-video synchronization
pub struct LipSync {
    /// Configuration
    config: LipSyncConfig,
    /// Audio buffer
    audio_buffer: VecDeque<MediaEntry>,
    /// Video buffer
    video_buffer: VecDeque<MediaEntry>,
    /// Media clock
    clock: Arc<Mutex<MediaClock>>,
    /// Measured audio-video delay in milliseconds
    av_delay_ms: i32,
    /// Audio clock rate in Hz
    audio_clock_rate: u32,
    /// Video clock rate in Hz
    video_clock_rate: u32,
    /// Last audio presentation time
    last_audio_presentation: Option<Instant>,
    /// Last video presentation time
    last_video_presentation: Option<Instant>,
    /// Synchronization stats
    stats: LipSyncStats,
}

/// Lip sync statistics
#[derive(Debug, Clone, Default)]
struct LipSyncStats {
    /// Number of audio frames processed
    audio_frames: u64,
    /// Number of video frames processed
    video_frames: u64,
    /// Number of audio frames dropped
    audio_dropped: u64,
    /// Number of video frames dropped
    video_dropped: u64,
    /// Maximum measured A/V delay in milliseconds
    max_delay_ms: i32,
    /// Average A/V delay in milliseconds
    avg_delay_ms: i32,
    /// Total synchronization adjustments made
    adjustments: u64,
}

impl LipSync {
    /// Create a new lip sync manager
    pub fn new(config: LipSyncConfig, clock: Arc<Mutex<MediaClock>>) -> Self {
        let audio_clock_rate = {
            let clock = clock.lock().unwrap();
            clock.clock_rate()
        };
        
        Self {
            config,
            audio_buffer: VecDeque::new(),
            video_buffer: VecDeque::new(),
            clock,
            av_delay_ms: 0,
            audio_clock_rate,
            video_clock_rate: audio_clock_rate, // Default to same as audio
            last_audio_presentation: None,
            last_video_presentation: None,
            stats: LipSyncStats::default(),
        }
    }
    
    /// Set the video clock rate
    pub fn set_video_clock_rate(&mut self, rate: u32) {
        self.video_clock_rate = rate;
    }
    
    /// Queue audio data
    pub fn queue_audio(&mut self, data: Bytes, timestamp: MediaTimestamp) -> Result<()> {
        self.stats.audio_frames += 1;
        let now = Instant::now();
        
        // Convert timestamp to the audio clock rate if needed
        let timestamp = if timestamp.clock_rate != self.audio_clock_rate {
            timestamp.with_clock_rate(self.audio_clock_rate)
        } else {
            timestamp
        };
        
        // Calculate presentation time
        let presentation_time = self.calculate_presentation_time(
            timestamp, 
            MediaType::Audio, 
            now
        )?;
        
        // Create entry
        let entry = MediaEntry {
            data,
            timestamp,
            capture_time: now,
            presentation_time: Some(presentation_time),
        };
        
        // Add to buffer
        self.audio_buffer.push_back(entry);
        
        // Cleanup old entries
        self.cleanup_buffers();
        
        Ok(())
    }
    
    /// Queue video data
    pub fn queue_video(&mut self, data: Bytes, timestamp: MediaTimestamp) -> Result<()> {
        self.stats.video_frames += 1;
        let now = Instant::now();
        
        // Convert timestamp to the video clock rate if needed
        let timestamp = if timestamp.clock_rate != self.video_clock_rate {
            timestamp.with_clock_rate(self.video_clock_rate)
        } else {
            timestamp
        };
        
        // Calculate presentation time
        let presentation_time = self.calculate_presentation_time(
            timestamp, 
            MediaType::Video, 
            now
        )?;
        
        // Create entry
        let entry = MediaEntry {
            data,
            timestamp,
            capture_time: now,
            presentation_time: Some(presentation_time),
        };
        
        // Add to buffer
        self.video_buffer.push_back(entry);
        
        // Cleanup old entries
        self.cleanup_buffers();
        
        Ok(())
    }
    
    /// Get audio data that is ready for presentation
    pub fn get_audio(&mut self) -> Option<Bytes> {
        let now = Instant::now();
        
        // Find the first entry ready for presentation
        let mut index = None;
        for (i, entry) in self.audio_buffer.iter().enumerate() {
            if let Some(pt) = entry.presentation_time {
                if pt <= now {
                    index = Some(i);
                    break;
                }
            }
        }
        
        // Return the entry if found
        if let Some(i) = index {
            let entry = self.audio_buffer.remove(i).unwrap();
            self.last_audio_presentation = Some(now);
            
            // Update A/V delay if we have video presentation info
            if let Some(last_video) = self.last_video_presentation {
                let delay_ms = now.duration_since(last_video).as_millis() as i32;
                self.update_av_delay(delay_ms);
            }
            
            Some(entry.data)
        } else {
            None
        }
    }
    
    /// Get video data that is ready for presentation
    pub fn get_video(&mut self) -> Option<Bytes> {
        let now = Instant::now();
        
        // Find the first entry ready for presentation
        let mut index = None;
        for (i, entry) in self.video_buffer.iter().enumerate() {
            if let Some(pt) = entry.presentation_time {
                if pt <= now {
                    index = Some(i);
                    break;
                }
            }
        }
        
        // Return the entry if found
        if let Some(i) = index {
            let entry = self.video_buffer.remove(i).unwrap();
            self.last_video_presentation = Some(now);
            
            // Update A/V delay if we have audio presentation info
            if let Some(last_audio) = self.last_audio_presentation {
                let delay_ms = last_audio.duration_since(now).as_millis() as i32;
                self.update_av_delay(delay_ms);
            }
            
            Some(entry.data)
        } else {
            None
        }
    }
    
    /// Calculate presentation time for a media frame
    fn calculate_presentation_time(
        &self,
        timestamp: MediaTimestamp,
        media_type: MediaType,
        now: Instant,
    ) -> Result<Instant> {
        let clock = self.clock.lock().unwrap();
        
        // Get the base presentation time
        let base_time = clock.rtp_to_time(timestamp.rtp_timestamp)?;
        
        // Apply media-specific buffering
        let buffer_ms = match media_type {
            MediaType::Audio => self.config.max_audio_buffer_ms,
            MediaType::Video => self.config.max_video_buffer_ms,
        };
        
        let buffer_time = Duration::from_millis(buffer_ms as u64);
        let presentation_time = base_time + buffer_time;
        
        // Apply A/V offset correction
        let target_offset_ms = self.config.target_av_offset_ms;
        let offset = Duration::from_millis(target_offset_ms.unsigned_abs() as u64);
        
        let adjusted_time = match media_type {
            MediaType::Audio => {
                if target_offset_ms > 0 {
                    // Audio should be ahead of video
                    presentation_time - offset
                } else {
                    // Audio should be behind video
                    presentation_time + offset
                }
            },
            MediaType::Video => {
                if target_offset_ms > 0 {
                    // Video should be behind audio
                    presentation_time + offset
                } else {
                    // Video should be ahead of audio
                    presentation_time - offset
                }
            }
        };
        
        // Ensure presentation time is in the future
        let final_time = if adjusted_time < now {
            trace!("Media would be late, scheduling immediately");
            now
        } else {
            adjusted_time
        };
        
        Ok(final_time)
    }
    
    /// Update the audio-video delay measurement
    fn update_av_delay(&mut self, delay_ms: i32) {
        // Update running average
        self.av_delay_ms = (self.av_delay_ms * 9 + delay_ms) / 10;
        
        // Update stats
        self.stats.avg_delay_ms = self.av_delay_ms;
        if delay_ms.abs() > self.stats.max_delay_ms {
            self.stats.max_delay_ms = delay_ms.abs();
        }
        
        // Adjust synchronization if needed
        if self.config.adaptive {
            let target = self.config.target_av_offset_ms;
            let window = self.config.sync_window_ms as i32;
            
            if (self.av_delay_ms - target).abs() > window {
                // Out of sync, adjust the target offset
                let adjustment = (self.av_delay_ms - target) / 2;
                self.config.target_av_offset_ms += adjustment;
                self.stats.adjustments += 1;
                
                debug!("Adjusting A/V sync: delay={}ms, adjustment={}ms, new target={}ms",
                       self.av_delay_ms, adjustment, self.config.target_av_offset_ms);
            }
        }
    }
    
    /// Cleanup old entries from buffers
    fn cleanup_buffers(&mut self) {
        let now = Instant::now();
        
        // Clean up audio buffer
        while !self.audio_buffer.is_empty() {
            let entry = &self.audio_buffer[0];
            
            // Check if entry is too old
            if let Some(pt) = entry.presentation_time {
                if pt < now.checked_sub(Duration::from_millis(500)).unwrap_or(now) {
                    self.audio_buffer.pop_front();
                    self.stats.audio_dropped += 1;
                    continue;
                }
            }
            
            break;
        }
        
        // Clean up video buffer
        while !self.video_buffer.is_empty() {
            let entry = &self.video_buffer[0];
            
            // Check if entry is too old
            if let Some(pt) = entry.presentation_time {
                if pt < now.checked_sub(Duration::from_millis(500)).unwrap_or(now) {
                    self.video_buffer.pop_front();
                    self.stats.video_dropped += 1;
                    continue;
                }
            }
            
            break;
        }
        
        // Limit buffer sizes
        let max_audio_ms = self.config.max_audio_buffer_ms;
        let max_video_ms = self.config.max_video_buffer_ms;
        
        while self.audio_buffer.len() > (max_audio_ms as usize / 20) {
            self.audio_buffer.pop_front();
            self.stats.audio_dropped += 1;
        }
        
        while self.video_buffer.len() > (max_video_ms as usize / 33) {
            self.video_buffer.pop_front();
            self.stats.video_dropped += 1;
        }
    }
    
    /// Get current A/V delay in milliseconds
    pub fn current_av_delay_ms(&self) -> i32 {
        self.av_delay_ms
    }
    
    /// Get the number of audio frames in buffer
    pub fn audio_buffer_size(&self) -> usize {
        self.audio_buffer.len()
    }
    
    /// Get the number of video frames in buffer
    pub fn video_buffer_size(&self) -> usize {
        self.video_buffer.len()
    }
    
    /// Reset the lip sync state
    pub fn reset(&mut self) {
        self.audio_buffer.clear();
        self.video_buffer.clear();
        self.av_delay_ms = 0;
        self.last_audio_presentation = None;
        self.last_video_presentation = None;
        self.stats = LipSyncStats::default();
        
        debug!("Lip sync reset");
    }
    
    /// Get statistics
    pub fn stats(&self) -> LipSyncStats {
        self.stats.clone()
    }
} 