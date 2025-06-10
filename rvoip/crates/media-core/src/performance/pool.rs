//! Object pooling for audio frames and media processing
//!
//! This module provides memory-efficient object pools to eliminate
//! allocations during real-time media processing.

use std::sync::{Arc, Mutex};
use std::collections::VecDeque;
use crate::types::AudioFrame;
use crate::performance::zero_copy::{ZeroCopyAudioFrame, SharedAudioBuffer};

/// Configuration for audio frame pool
#[derive(Debug, Clone)]
pub struct PoolConfig {
    /// Initial pool size
    pub initial_size: usize,
    /// Maximum pool size (0 = unlimited)
    pub max_size: usize,
    /// Sample rate for pooled frames
    pub sample_rate: u32,
    /// Number of channels for pooled frames
    pub channels: u8,
    /// Samples per frame
    pub samples_per_frame: usize,
}

impl Default for PoolConfig {
    fn default() -> Self {
        Self {
            initial_size: 16,
            max_size: 64,
            sample_rate: 8000,
            channels: 1,
            samples_per_frame: 160, // 20ms at 8kHz
        }
    }
}

/// A pooled audio frame that returns to the pool when dropped
pub struct PooledAudioFrame {
    frame: ZeroCopyAudioFrame,
    pool: Arc<AudioFramePool>,
    returned: bool,
}

impl PooledAudioFrame {
    /// Create a new pooled frame
    fn new(frame: ZeroCopyAudioFrame, pool: Arc<AudioFramePool>) -> Self {
        Self {
            frame,
            pool,
            returned: false,
        }
    }
    
    /// Get the underlying zero-copy frame
    pub fn frame(&self) -> &ZeroCopyAudioFrame {
        &self.frame
    }
    
    /// Get mutable access to the frame
    pub fn frame_mut(&mut self) -> &mut ZeroCopyAudioFrame {
        &mut self.frame
    }
    
    /// Convert to owned ZeroCopyAudioFrame (consumes the pooled frame)
    pub fn into_frame(mut self) -> ZeroCopyAudioFrame {
        self.returned = true; // Prevent return to pool
        self.frame.clone()
    }
    
    /// Manually return to pool (normally done automatically on drop)
    pub fn return_to_pool(mut self) {
        if !self.returned {
            self.pool.return_frame(self.frame.clone());
            self.returned = true;
        }
    }
}

impl Drop for PooledAudioFrame {
    fn drop(&mut self) {
        if !self.returned {
            self.pool.return_frame(self.frame.clone());
        }
    }
}

impl std::ops::Deref for PooledAudioFrame {
    type Target = ZeroCopyAudioFrame;
    
    fn deref(&self) -> &Self::Target {
        &self.frame
    }
}

impl std::ops::DerefMut for PooledAudioFrame {
    fn deref_mut(&mut self) -> &mut Self::Target {
        &mut self.frame
    }
}

/// Statistics for the audio frame pool
#[derive(Debug, Clone)]
pub struct PoolStats {
    /// Current pool size
    pub pool_size: usize,
    /// Number of frames allocated
    pub allocated_count: u64,
    /// Number of frames returned
    pub returned_count: u64,
    /// Number of pool hits (reused frames)
    pub pool_hits: u64,
    /// Number of pool misses (new allocations)
    pub pool_misses: u64,
    /// Maximum pool size reached
    pub max_pool_size: usize,
}

/// Object pool for zero-copy audio frames
#[derive(Debug)]
pub struct AudioFramePool {
    config: PoolConfig,
    pool: Mutex<VecDeque<ZeroCopyAudioFrame>>,
    stats: Mutex<PoolStats>,
}

impl AudioFramePool {
    /// Create a new audio frame pool
    pub fn new(config: PoolConfig) -> Arc<Self> {
        let pool_size = config.initial_size;
        let mut frames = VecDeque::with_capacity(pool_size);
        
        // Pre-populate pool with frames
        for _ in 0..pool_size {
            let samples = vec![0i16; config.samples_per_frame * config.channels as usize];
            let frame = ZeroCopyAudioFrame::new(samples, config.sample_rate, config.channels, 0);
            frames.push_back(frame);
        }
        
        let stats = PoolStats {
            pool_size,
            allocated_count: 0,
            returned_count: 0,
            pool_hits: 0,
            pool_misses: 0,
            max_pool_size: pool_size,
        };
        
        Arc::new(Self {
            config,
            pool: Mutex::new(frames),
            stats: Mutex::new(stats),
        })
    }
    
    /// Get a frame from the pool (reuse existing or allocate new)
    pub fn get_frame(self: &Arc<Self>) -> PooledAudioFrame {
        let mut pool = self.pool.lock().unwrap();
        let mut stats = self.stats.lock().unwrap();
        
        if let Some(mut frame) = pool.pop_front() {
            // Reuse existing frame from pool
            frame.timestamp = 0; // Reset timestamp
            stats.pool_hits += 1;
            stats.allocated_count += 1;
            
            PooledAudioFrame::new(frame, self.clone())
        } else {
            // Pool empty, allocate new frame
            let samples = vec![0i16; self.config.samples_per_frame * self.config.channels as usize];
            let frame = ZeroCopyAudioFrame::new(
                samples,
                self.config.sample_rate,
                self.config.channels,
                0,
            );
            
            stats.pool_misses += 1;
            stats.allocated_count += 1;
            
            PooledAudioFrame::new(frame, self.clone())
        }
    }
    
    /// Get a frame with specific parameters
    pub fn get_frame_with_params(
        self: &Arc<Self>,
        sample_rate: u32,
        channels: u8,
        sample_count: usize,
    ) -> PooledAudioFrame {
        let total_samples = sample_count * channels as usize;
        
        // Try to reuse a frame if dimensions match
        if sample_rate == self.config.sample_rate
            && channels == self.config.channels
            && total_samples <= self.config.samples_per_frame * self.config.channels as usize
        {
            // Can reuse from pool
            let mut pooled = self.get_frame();
            
            // Truncate buffer if needed
            if total_samples < pooled.frame().buffer.len() {
                if let Some(sliced) = pooled.frame().buffer.slice(0, total_samples) {
                    pooled.frame_mut().buffer = sliced;
                }
            }
            
            pooled.frame_mut().sample_rate = sample_rate;
            pooled.frame_mut().channels = channels;
            
            pooled
        } else {
            // Need different dimensions, allocate new
            let samples = vec![0i16; total_samples];
            let frame = ZeroCopyAudioFrame::new(samples, sample_rate, channels, 0);
            
            let mut stats = self.stats.lock().unwrap();
            stats.pool_misses += 1;
            stats.allocated_count += 1;
            
            PooledAudioFrame::new(frame, self.clone())
        }
    }
    
    /// Return a frame to the pool
    fn return_frame(&self, frame: ZeroCopyAudioFrame) {
        let mut pool = self.pool.lock().unwrap();
        let mut stats = self.stats.lock().unwrap();
        
        // Only return to pool if it matches config and pool isn't full
        if frame.sample_rate == self.config.sample_rate
            && frame.channels == self.config.channels
            && frame.buffer.len() == self.config.samples_per_frame * self.config.channels as usize
            && (self.config.max_size == 0 || pool.len() < self.config.max_size)
        {
            pool.push_back(frame);
            stats.returned_count += 1;
            stats.pool_size = pool.len();
            stats.max_pool_size = stats.max_pool_size.max(pool.len());
        }
        // Otherwise, frame is dropped (garbage collected)
    }
    
    /// Get current pool statistics
    pub fn get_stats(&self) -> PoolStats {
        let pool = self.pool.lock().unwrap();
        let mut stats = self.stats.lock().unwrap();
        stats.pool_size = pool.len();
        stats.clone()
    }
    
    /// Clear the pool and reset statistics
    pub fn clear(&self) {
        let mut pool = self.pool.lock().unwrap();
        let mut stats = self.stats.lock().unwrap();
        
        pool.clear();
        *stats = PoolStats {
            pool_size: 0,
            allocated_count: 0,
            returned_count: 0,
            pool_hits: 0,
            pool_misses: 0,
            max_pool_size: 0,
        };
    }
    
    /// Pre-warm the pool with additional frames
    pub fn prewarm(&self, additional_frames: usize) {
        let mut pool = self.pool.lock().unwrap();
        
        for _ in 0..additional_frames {
            let samples = vec![0i16; self.config.samples_per_frame * self.config.channels as usize];
            let frame = ZeroCopyAudioFrame::new(
                samples,
                self.config.sample_rate,
                self.config.channels,
                0,
            );
            pool.push_back(frame);
        }
        
        let mut stats = self.stats.lock().unwrap();
        stats.pool_size = pool.len();
        stats.max_pool_size = stats.max_pool_size.max(pool.len());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_pool_creation() {
        let config = PoolConfig::default();
        let pool = AudioFramePool::new(config.clone());
        let stats = pool.get_stats();
        
        assert_eq!(stats.pool_size, config.initial_size);
        assert_eq!(stats.allocated_count, 0);
        assert_eq!(stats.pool_hits, 0);
        assert_eq!(stats.pool_misses, 0);
    }
    
    #[test]
    fn test_pool_get_frame() {
        let config = PoolConfig::default();
        let pool = AudioFramePool::new(config.clone());
        
        // Get a frame (should be pool hit)
        let frame1 = pool.get_frame();
        assert_eq!(frame1.sample_rate, config.sample_rate);
        assert_eq!(frame1.channels, config.channels);
        
        let stats = pool.get_stats();
        assert_eq!(stats.pool_hits, 1);
        assert_eq!(stats.pool_misses, 0);
        assert_eq!(stats.allocated_count, 1);
        assert_eq!(stats.pool_size, config.initial_size - 1);
    }
    
    #[test]
    fn test_pool_return_frame() {
        let config = PoolConfig::default();
        let pool = AudioFramePool::new(config.clone());
        
        {
            let _frame = pool.get_frame(); // Frame will be returned on drop
        }
        
        let stats = pool.get_stats();
        assert_eq!(stats.returned_count, 1);
        assert_eq!(stats.pool_size, config.initial_size); // Back to original size
    }
    
    #[test]
    fn test_pool_exhaustion() {
        let mut config = PoolConfig::default();
        config.initial_size = 2;
        let pool = AudioFramePool::new(config);
        
        // Exhaust the pool
        let _frame1 = pool.get_frame();
        let _frame2 = pool.get_frame();
        let frame3 = pool.get_frame(); // Should trigger new allocation
        
        let stats = pool.get_stats();
        assert_eq!(stats.pool_hits, 2);
        assert_eq!(stats.pool_misses, 1);
        assert_eq!(stats.pool_size, 0);
    }
    
    #[test]
    fn test_pool_with_different_params() {
        let config = PoolConfig::default();
        let pool = AudioFramePool::new(config);
        
        // Get frame with different parameters
        let frame = pool.get_frame_with_params(16000, 2, 320);
        assert_eq!(frame.sample_rate, 16000);
        assert_eq!(frame.channels, 2);
        
        let stats = pool.get_stats();
        assert_eq!(stats.pool_misses, 1); // Should miss since params differ
    }
    
    #[test]
    fn test_pooled_frame_auto_return() {
        let config = PoolConfig::default();
        let pool = AudioFramePool::new(config.clone());
        
        let initial_stats = pool.get_stats();
        
        {
            let _frame = pool.get_frame();
            // Frame automatically returned when it goes out of scope
        }
        
        let final_stats = pool.get_stats();
        assert_eq!(final_stats.returned_count, initial_stats.returned_count + 1);
        assert_eq!(final_stats.pool_size, config.initial_size);
    }
    
    #[test]
    fn test_pool_manual_return() {
        let config = PoolConfig::default();
        let pool = AudioFramePool::new(config.clone());
        
        let frame = pool.get_frame();
        frame.return_to_pool(); // Manual return
        
        let stats = pool.get_stats();
        assert_eq!(stats.returned_count, 1);
        assert_eq!(stats.pool_size, config.initial_size);
    }
    
    #[test]
    fn test_pool_stats_tracking() {
        let config = PoolConfig::default();
        let pool = AudioFramePool::new(config);
        
        // Perform various operations
        let _frame1 = pool.get_frame();
        let _frame2 = pool.get_frame();
        drop(_frame1); // Return to pool
        let _frame3 = pool.get_frame(); // Should reuse
        
        let stats = pool.get_stats();
        assert_eq!(stats.allocated_count, 3);
        assert_eq!(stats.returned_count, 1);
        assert_eq!(stats.pool_hits, 3); // All from pool initially
        assert_eq!(stats.pool_misses, 0);
    }
} 