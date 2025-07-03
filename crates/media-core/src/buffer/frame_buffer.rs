//! Frame Buffer
//!
//! This module provides a frame buffer specifically designed for storing
//! audio frames with timing information and ordered retrieval.

use std::collections::BTreeMap;
use tokio::sync::RwLock;
use tracing::{debug, trace};

use crate::error::{Result, BufferError};
use crate::types::AudioFrame;

/// Configuration for frame buffer
#[derive(Debug, Clone)]
pub struct FrameBufferConfig {
    /// Maximum number of frames to store
    pub max_frames: usize,
    /// Frame duration in milliseconds
    pub frame_duration_ms: u32,
    /// Enable statistics collection
    pub enable_statistics: bool,
}

impl Default for FrameBufferConfig {
    fn default() -> Self {
        Self {
            max_frames: 200,       // ~4 seconds at 20ms frames
            frame_duration_ms: 20, // Standard 20ms frames
            enable_statistics: true,
        }
    }
}

/// Frame entry with timing information
#[derive(Debug, Clone)]
struct FrameEntry {
    /// Audio frame
    frame: AudioFrame,
    /// Insert timestamp (for ordering)
    insert_time: std::time::Instant,
    /// Frame timestamp (from RTP or local)
    frame_timestamp: u32,
}

/// Frame buffer statistics
#[derive(Debug, Clone, Default)]
pub struct FrameBufferStats {
    /// Frames stored
    pub frames_stored: u64,
    /// Frames retrieved
    pub frames_retrieved: u64,
    /// Frames dropped (overflow)
    pub frames_dropped: u64,
    /// Current frame count
    pub current_count: usize,
    /// Maximum frames reached
    pub peak_count: usize,
}

/// A buffer for storing audio frames with timing information
pub struct FrameBuffer {
    /// Configuration
    config: FrameBufferConfig,
    /// Buffered frames (indexed by timestamp)
    frames: RwLock<BTreeMap<u32, FrameEntry>>,
    /// Statistics
    stats: RwLock<FrameBufferStats>,
}

impl FrameBuffer {
    /// Create a new frame buffer
    pub fn new(config: FrameBufferConfig) -> Self {
        debug!("Creating FrameBuffer with config: {:?}", config);
        
        Self {
            config,
            frames: RwLock::new(BTreeMap::new()),
            stats: RwLock::new(FrameBufferStats::default()),
        }
    }
    
    /// Store a frame in the buffer
    pub async fn store_frame(&self, frame: AudioFrame, timestamp: u32) -> Result<()> {
        let entry = FrameEntry {
            frame,
            insert_time: std::time::Instant::now(),
            frame_timestamp: timestamp,
        };
        
        // Check for overflow
        {
            let frames = self.frames.read().await;
            if frames.len() >= self.config.max_frames {
                // Remove oldest frame
                if let Some((&oldest_ts, _)) = frames.iter().next() {
                    drop(frames); // Release read lock
                    self.remove_frame(oldest_ts).await;
                    
                    let mut stats = self.stats.write().await;
                    stats.frames_dropped += 1;
                    
                    debug!("Frame buffer overflow, dropped frame at timestamp {}", oldest_ts);
                }
            }
        }
        
        // Store the frame
        {
            let mut frames = self.frames.write().await;
            frames.insert(timestamp, entry);
        }
        
        // Update statistics
        if self.config.enable_statistics {
            let mut stats = self.stats.write().await;
            stats.frames_stored += 1;
            
            let frame_count = self.frames.read().await.len();
            stats.current_count = frame_count;
            if frame_count > stats.peak_count {
                stats.peak_count = frame_count;
            }
        }
        
        trace!("Stored frame at timestamp {}", timestamp);
        Ok(())
    }
    
    /// Retrieve a frame by timestamp
    pub async fn get_frame(&self, timestamp: u32) -> Option<AudioFrame> {
        let frame = {
            let mut frames = self.frames.write().await;
            frames.remove(&timestamp).map(|entry| entry.frame)
        };
        
        if frame.is_some() {
            // Update statistics
            if self.config.enable_statistics {
                let mut stats = self.stats.write().await;
                stats.frames_retrieved += 1;
                stats.current_count = self.frames.read().await.len();
            }
            
            trace!("Retrieved frame at timestamp {}", timestamp);
        }
        
        frame
    }
    
    /// Get the oldest frame (by timestamp)
    pub async fn get_oldest_frame(&self) -> Option<(u32, AudioFrame)> {
        let (timestamp, frame) = {
            let mut frames = self.frames.write().await;
            if let Some((&ts, _)) = frames.iter().next() {
                let entry = frames.remove(&ts)?;
                Some((ts, entry.frame))
            } else {
                None
            }
        }?;
        
        // Update statistics
        if self.config.enable_statistics {
            let mut stats = self.stats.write().await;
            stats.frames_retrieved += 1;
            stats.current_count = self.frames.read().await.len();
        }
        
        trace!("Retrieved oldest frame at timestamp {}", timestamp);
        Some((timestamp, frame))
    }
    
    /// Get the newest frame (by timestamp)
    pub async fn get_newest_frame(&self) -> Option<(u32, AudioFrame)> {
        let (timestamp, frame) = {
            let mut frames = self.frames.write().await;
            if let Some((&ts, _)) = frames.iter().last() {
                let entry = frames.remove(&ts)?;
                Some((ts, entry.frame))
            } else {
                None
            }
        }?;
        
        // Update statistics
        if self.config.enable_statistics {
            let mut stats = self.stats.write().await;
            stats.frames_retrieved += 1;
            stats.current_count = self.frames.read().await.len();
        }
        
        trace!("Retrieved newest frame at timestamp {}", timestamp);
        Some((timestamp, frame))
    }
    
    /// Get frames in a timestamp range
    pub async fn get_frames_in_range(&self, start_ts: u32, end_ts: u32) -> Vec<(u32, AudioFrame)> {
        let mut result = Vec::new();
        
        {
            let mut frames = self.frames.write().await;
            let timestamps_to_remove: Vec<u32> = frames
                .range(start_ts..=end_ts)
                .map(|(&ts, _)| ts)
                .collect();
            
            for ts in timestamps_to_remove {
                if let Some(entry) = frames.remove(&ts) {
                    result.push((ts, entry.frame));
                }
            }
        }
        
        // Update statistics
        if self.config.enable_statistics && !result.is_empty() {
            let mut stats = self.stats.write().await;
            stats.frames_retrieved += result.len() as u64;
            stats.current_count = self.frames.read().await.len();
        }
        
        trace!("Retrieved {} frames in range {}..{}", result.len(), start_ts, end_ts);
        result
    }
    
    /// Remove a frame by timestamp
    pub async fn remove_frame(&self, timestamp: u32) -> bool {
        let removed = {
            let mut frames = self.frames.write().await;
            frames.remove(&timestamp).is_some()
        };
        
        if removed {
            trace!("Removed frame at timestamp {}", timestamp);
        }
        
        removed
    }
    
    /// Check if buffer contains a frame at timestamp
    pub async fn contains_frame(&self, timestamp: u32) -> bool {
        let frames = self.frames.read().await;
        frames.contains_key(&timestamp)
    }
    
    /// Get the number of frames in buffer
    pub async fn frame_count(&self) -> usize {
        self.frames.read().await.len()
    }
    
    /// Check if buffer is empty
    pub async fn is_empty(&self) -> bool {
        self.frames.read().await.is_empty()
    }
    
    /// Get the timestamp range of buffered frames
    pub async fn get_timestamp_range(&self) -> Option<(u32, u32)> {
        let frames = self.frames.read().await;
        if let (Some((&first, _)), Some((&last, _))) = (frames.iter().next(), frames.iter().last()) {
            Some((first, last))
        } else {
            None
        }
    }
    
    /// Clear all frames from buffer
    pub async fn clear(&self) {
        {
            let mut frames = self.frames.write().await;
            frames.clear();
        }
        
        if self.config.enable_statistics {
            let mut stats = self.stats.write().await;
            stats.current_count = 0;
        }
        
        debug!("Frame buffer cleared");
    }
    
    /// Get buffer statistics
    pub async fn get_statistics(&self) -> FrameBufferStats {
        let mut stats = self.stats.read().await.clone();
        stats.current_count = self.frame_count().await;
        stats
    }
    
    /// Get frames older than the specified age
    pub async fn get_frames_older_than(&self, max_age: std::time::Duration) -> Vec<(u32, AudioFrame)> {
        let cutoff_time = std::time::Instant::now() - max_age;
        let mut result = Vec::new();
        
        {
            let mut frames = self.frames.write().await;
            let timestamps_to_remove: Vec<u32> = frames
                .iter()
                .filter(|(_, entry)| entry.insert_time < cutoff_time)
                .map(|(&ts, _)| ts)
                .collect();
            
            for ts in timestamps_to_remove {
                if let Some(entry) = frames.remove(&ts) {
                    result.push((ts, entry.frame));
                }
            }
        }
        
        if !result.is_empty() {
            debug!("Removed {} frames older than {:?}", result.len(), max_age);
        }
        
        result
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::types::SampleRate;
    
    #[tokio::test]
    async fn test_frame_buffer_creation() {
        let config = FrameBufferConfig::default();
        let buffer = FrameBuffer::new(config);
        
        assert_eq!(buffer.frame_count().await, 0);
        assert!(buffer.is_empty().await);
        assert!(buffer.get_timestamp_range().await.is_none());
    }
    
    #[tokio::test]
    async fn test_store_and_retrieve_frame() {
        let buffer = FrameBuffer::new(FrameBufferConfig::default());
        
        let frame = AudioFrame::new(vec![100; 160], 8000, 1, 1000);
        let timestamp = 8000;
        
        buffer.store_frame(frame.clone(), timestamp).await.unwrap();
        assert_eq!(buffer.frame_count().await, 1);
        assert!(buffer.contains_frame(timestamp).await);
        
        let retrieved = buffer.get_frame(timestamp).await;
        assert!(retrieved.is_some());
        assert_eq!(buffer.frame_count().await, 0);
        assert!(!buffer.contains_frame(timestamp).await);
    }
    
    #[tokio::test]
    async fn test_oldest_newest_frames() {
        let buffer = FrameBuffer::new(FrameBufferConfig::default());
        
        // Store frames with different timestamps
        let frame1 = AudioFrame::new(vec![100; 160], 8000, 1, 1000);
        let frame2 = AudioFrame::new(vec![200; 160], 8000, 1, 1001);
        let frame3 = AudioFrame::new(vec![300; 160], 8000, 1, 1002);
        
        buffer.store_frame(frame2, 8160).await.unwrap(); // Middle
        buffer.store_frame(frame1, 8000).await.unwrap(); // Oldest
        buffer.store_frame(frame3, 8320).await.unwrap(); // Newest
        
        // Get oldest
        let (ts, _) = buffer.get_oldest_frame().await.unwrap();
        assert_eq!(ts, 8000);
        
        // Get newest
        let (ts, _) = buffer.get_newest_frame().await.unwrap();
        assert_eq!(ts, 8320);
        
        assert_eq!(buffer.frame_count().await, 1); // One frame left
    }
    
    #[tokio::test]
    async fn test_frame_range_retrieval() {
        let buffer = FrameBuffer::new(FrameBufferConfig::default());
        
        // Store multiple frames
        for i in 0..10 {
            let frame = AudioFrame::new(vec![i as i16; 160], 8000, 1, i as u32);
            buffer.store_frame(frame, 8000 + i * 160).await.unwrap();
        }
        
        // Get frames in range
        let frames = buffer.get_frames_in_range(8160, 8480).await;
        assert_eq!(frames.len(), 3); // Frames at 8160, 8320, 8480
        
        assert_eq!(buffer.frame_count().await, 7); // 7 frames left
    }
    
    #[tokio::test]
    async fn test_buffer_overflow() {
        let config = FrameBufferConfig {
            max_frames: 3,
            ..Default::default()
        };
        let buffer = FrameBuffer::new(config);
        
        // Store more frames than capacity
        for i in 0..5 {
            let frame = AudioFrame::new(vec![i as i16; 160], 8000, 1, i as u32);
            buffer.store_frame(frame, 8000 + i * 160).await.unwrap();
        }
        
        // Should only have max_frames
        assert_eq!(buffer.frame_count().await, 3);
        
        let stats = buffer.get_statistics().await;
        assert_eq!(stats.frames_stored, 5);
        assert_eq!(stats.frames_dropped, 2);
    }
} 