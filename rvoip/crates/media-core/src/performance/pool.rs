//! High-performance memory pooling for audio processing
//!
//! This module provides efficient memory pooling to reduce allocations
//! during real-time audio processing operations.

use std::sync::{Arc, Mutex};
use std::collections::VecDeque;
use std::time::Instant;
use tracing::{debug, warn};
use crate::types::{AudioFrame, SampleRate};
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
    
    /// Get a reference to the samples
    pub fn samples(&self) -> &[i16] {
        self.frame.samples()
    }
    
    /// Get a mutable reference to the samples for zero-copy processing
    pub fn samples_mut(&mut self) -> &mut [i16] {
        // For zero-copy processing, we need mutable access to the underlying samples
        // This is safe because PooledAudioFrame has exclusive access during processing
        unsafe {
            let samples_ptr = self.frame.samples().as_ptr() as *mut i16;
            std::slice::from_raw_parts_mut(samples_ptr, self.frame.samples().len())
        }
    }
    
    /// Get frame metadata
    pub fn sample_rate(&self) -> u32 {
        self.frame.sample_rate
    }
    
    /// Get number of channels
    pub fn channels(&self) -> u8 {
        self.frame.channels
    }
    
    /// Get timestamp
    pub fn timestamp(&self) -> u32 {
        self.frame.timestamp
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

/// Pool statistics
#[derive(Debug, Clone, Default)]
pub struct PoolStats {
    /// Current pool size
    pub pool_size: usize,
    /// Number of frames allocated
    pub allocated_count: usize,
    /// Number of frames returned to pool
    pub returned_count: usize,
    /// Number of pool hits (reused frames)
    pub pool_hits: usize,
    /// Number of pool misses (new allocations)
    pub pool_misses: usize,
    /// Maximum pool size reached
    pub max_pool_size: usize,
    
    // NEW: Additional fields for RtpBufferPool compatibility
    /// Total buffers allocated
    pub total_allocated: usize,
    /// Available buffers in pool
    pub available: usize,
    /// Cache hits (buffer reuse)
    pub cache_hits: usize,
    /// Cache misses (new buffer creation)
    pub cache_misses: usize,
    /// Pool exhaustion events
    pub pool_exhausted: usize,
}

impl PoolStats {
    /// Create new empty statistics
    pub fn new() -> Self {
        Self::default()
    }
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
            total_allocated: 0,
            available: 0,
            cache_hits: 0,
            cache_misses: 0,
            pool_exhausted: 0,
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
            total_allocated: 0,
            available: 0,
            cache_hits: 0,
            cache_misses: 0,
            pool_exhausted: 0,
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

/// Pool for RTP output buffers (for zero-copy encoding)
pub struct RtpBufferPool {
    buffers: Mutex<VecDeque<Vec<u8>>>,
    buffer_size: usize,
    initial_count: usize,
    max_count: usize,
    stats: Arc<Mutex<PoolStats>>,
}

/// Pooled RTP output buffer that automatically returns to pool on drop
pub struct PooledRtpBuffer {
    buffer: Option<Vec<u8>>,
    pool: Arc<RtpBufferPool>,
    capacity: usize,
}

impl RtpBufferPool {
    /// Create a new RTP buffer pool
    pub fn new(buffer_size: usize, initial_count: usize, max_count: usize) -> Arc<Self> {
        let pool = Arc::new(Self {
            buffers: Mutex::new(VecDeque::with_capacity(initial_count)),
            buffer_size,
            initial_count,
            max_count,
            stats: Arc::new(Mutex::new(PoolStats::new())),
        });
        
        // Pre-allocate initial buffers
        {
            let mut buffers = pool.buffers.lock().unwrap();
            for _ in 0..initial_count {
                let mut buffer = Vec::with_capacity(buffer_size);
                buffer.resize(buffer_size, 0);
                buffers.push_back(buffer);
            }
            
            let mut stats = pool.stats.lock().unwrap();
            stats.total_allocated = initial_count;
            stats.available = initial_count;
        }
        
        debug!("Created RTP buffer pool: size={}, initial={}, max={}", 
               buffer_size, initial_count, max_count);
        
        pool
    }
    
    /// Get a buffer from the pool
    pub fn get_buffer(self: &Arc<Self>) -> PooledRtpBuffer {
        let buffer = {
            let mut buffers = self.buffers.lock().unwrap();
            let mut stats = self.stats.lock().unwrap();
            
            if let Some(mut buffer) = buffers.pop_front() {
                // Reuse existing buffer
                buffer.clear();
                buffer.resize(self.buffer_size, 0);
                stats.cache_hits += 1;
                stats.available = stats.available.saturating_sub(1);
                buffer
            } else if stats.total_allocated < self.max_count {
                // Create new buffer
                let mut buffer = Vec::with_capacity(self.buffer_size);
                buffer.resize(self.buffer_size, 0);
                stats.total_allocated += 1;
                stats.cache_misses += 1;
                buffer
            } else {
                // Pool exhausted, create temporary buffer
                let mut buffer = Vec::with_capacity(self.buffer_size);
                buffer.resize(self.buffer_size, 0);
                stats.pool_exhausted += 1;
                warn!("RTP buffer pool exhausted, creating temporary buffer");
                buffer
            }
        };
        
        PooledRtpBuffer {
            buffer: Some(buffer),
            pool: self.clone(),
            capacity: self.buffer_size,
        }
    }
    
    /// Return a buffer to the pool
    fn return_buffer(&self, buffer: Vec<u8>) {
        let mut buffers = self.buffers.lock().unwrap();
        let mut stats = self.stats.lock().unwrap();
        
        if buffers.len() < self.initial_count && buffer.capacity() == self.buffer_size {
            buffers.push_back(buffer);
            stats.available += 1;
        } else {
            // Don't keep oversized or excess buffers
            stats.total_allocated = stats.total_allocated.saturating_sub(1);
        }
    }
    
    /// Get pool statistics
    pub fn get_stats(&self) -> PoolStats {
        let stats = self.stats.lock().unwrap();
        stats.clone()
    }
    
    /// Reset pool statistics
    pub fn reset_stats(&self) {
        let mut stats = self.stats.lock().unwrap();
        *stats = PoolStats::new();
    }
}

impl PooledRtpBuffer {
    /// Get mutable slice for writing encoded data
    pub fn as_mut(&mut self) -> &mut [u8] {
        self.buffer.as_mut().unwrap()
    }
    
    /// Get slice for reading encoded data
    pub fn as_slice(&self) -> &[u8] {
        self.buffer.as_ref().unwrap()
    }
    
    /// Create a Bytes slice from the first `len` bytes
    pub fn slice(&self, len: usize) -> bytes::Bytes {
        let buffer = self.buffer.as_ref().unwrap();
        bytes::Bytes::copy_from_slice(&buffer[..len.min(buffer.len())])
    }
    
    /// Get the capacity of this buffer
    pub fn capacity(&self) -> usize {
        self.capacity
    }
    
    /// Resize the buffer to the specified length
    pub fn resize(&mut self, new_len: usize) {
        if let Some(ref mut buffer) = self.buffer {
            buffer.resize(new_len.min(self.capacity), 0);
        }
    }
}

impl Drop for PooledRtpBuffer {
    fn drop(&mut self) {
        if let Some(buffer) = self.buffer.take() {
            self.pool.return_buffer(buffer);
        }
    }
}

impl std::ops::Deref for PooledRtpBuffer {
    type Target = [u8];
    
    fn deref(&self) -> &Self::Target {
        self.buffer.as_ref().unwrap()
    }
}

impl std::ops::DerefMut for PooledRtpBuffer {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.buffer.as_mut().unwrap()
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