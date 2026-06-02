//! High-performance memory pooling for audio processing
//!
//! This module provides efficient memory pooling to reduce allocations
//! during real-time audio processing operations.

use crate::performance::zero_copy::ZeroCopyAudioFrame;
use crossbeam_queue::ArrayQueue;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::Arc;
use tracing::{debug, warn};

/// Cache-line-aligned `AtomicUsize` wrapper used for hot-path pool
/// counters. The previous `parking_lot::Mutex<PoolStats>` was the
/// dominant contention point on `pipeline_concurrent/{8,16}` after
/// the metrics gate (C17) and lock-free pool storage (C15) landed —
/// removing the lock without padding caused MESI thrash across the
/// dense `PoolStats` struct. 64-byte alignment puts each counter on
/// its own cache line so 16 worker threads can update independent
/// fields without bouncing the line between cores.
#[derive(Debug, Default)]
#[repr(align(64))]
struct PaddedCounter(AtomicUsize);

impl PaddedCounter {
    const fn new() -> Self {
        Self(AtomicUsize::new(0))
    }
    #[inline]
    fn inc(&self) {
        self.0.fetch_add(1, Ordering::Relaxed);
    }
    #[inline]
    #[allow(dead_code)]
    fn add(&self, n: usize) {
        self.0.fetch_add(n, Ordering::Relaxed);
    }
    #[inline]
    fn dec(&self) {
        // Saturating subtract via fetch_update — under-flow guard
        // matches the prior `saturating_sub` semantics.
        let _ = self
            .0
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |v| {
                Some(v.saturating_sub(1))
            });
    }
    #[inline]
    fn set(&self, v: usize) {
        self.0.store(v, Ordering::Relaxed);
    }
    #[inline]
    fn get(&self) -> usize {
        self.0.load(Ordering::Relaxed)
    }
    /// Update `self` to the max of its current value and `v`. Lock-free.
    #[inline]
    fn update_max(&self, v: usize) {
        let _ = self
            .0
            .fetch_update(Ordering::Relaxed, Ordering::Relaxed, |cur| Some(cur.max(v)));
    }
}

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

/// Object pool for zero-copy audio frames.
///
/// Storage is a lock-free MPMC `ArrayQueue` so the per-frame
/// `get`/`return` does not block other concurrent tasks — previously
/// every per-frame call took a `std::Mutex<VecDeque>` guard, which
/// was the dominant ceiling on `pipeline_concurrent/{8,16}` after the
/// codec mutex was removed. Stats are now lock-free
/// cache-line-padded atomics (see `PaddedCounter`) — the earlier
/// `parking_lot::Mutex<PoolStats>` showed up as the next bottleneck
/// once the metrics RwLock was gated off in C17. Per-call increments
/// on independent padded counters scaled cleanly to 16 worker
/// threads in the bench (~-54% time at /16).
#[derive(Debug)]
pub struct AudioFramePool {
    config: PoolConfig,
    pool: ArrayQueue<ZeroCopyAudioFrame>,
    pool_hits: PaddedCounter,
    pool_misses: PaddedCounter,
    allocated_count: PaddedCounter,
    returned_count: PaddedCounter,
    max_pool_size: PaddedCounter,
}

impl AudioFramePool {
    /// Create a new audio frame pool
    pub fn new(config: PoolConfig) -> Arc<Self> {
        // `ArrayQueue` is bounded; size it to the configured maximum
        // (or to `initial_size * 4` as a sane upper bound when
        // `max_size == 0` / unlimited). Once full, returned frames
        // are dropped instead of pooled — same observable behavior
        // as the previous `pool.len() < max_size` guard.
        let capacity = if config.max_size == 0 {
            config.initial_size.max(1).saturating_mul(4)
        } else {
            config.max_size.max(config.initial_size).max(1)
        };
        let queue = ArrayQueue::new(capacity);
        let pool_size = config.initial_size;

        // Pre-populate pool with frames
        for _ in 0..pool_size {
            let samples = vec![0i16; config.samples_per_frame * config.channels as usize];
            let frame = ZeroCopyAudioFrame::new(samples, config.sample_rate, config.channels, 0);
            // Push can only fail if the queue is full — impossible
            // here since we just sized capacity ≥ initial_size.
            let _ = queue.push(frame);
        }

        let max_pool_size = PaddedCounter::new();
        max_pool_size.set(pool_size);

        Arc::new(Self {
            config,
            pool: queue,
            pool_hits: PaddedCounter::new(),
            pool_misses: PaddedCounter::new(),
            allocated_count: PaddedCounter::new(),
            returned_count: PaddedCounter::new(),
            max_pool_size,
        })
    }

    /// Get a frame from the pool (reuse existing or allocate new).
    ///
    /// Stats updates are now lock-free atomic increments on
    /// independently cache-line-padded counters — no shared lock or
    /// shared cache line in the per-frame hot path.
    pub fn get_frame(self: &Arc<Self>) -> PooledAudioFrame {
        if let Some(mut frame) = self.pool.pop() {
            frame.timestamp = 0;
            self.pool_hits.inc();
            self.allocated_count.inc();
            PooledAudioFrame::new(frame, self.clone())
        } else {
            let samples = vec![0i16; self.config.samples_per_frame * self.config.channels as usize];
            let frame =
                ZeroCopyAudioFrame::new(samples, self.config.sample_rate, self.config.channels, 0);
            self.pool_misses.inc();
            self.allocated_count.inc();
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
            self.pool_misses.inc();
            self.allocated_count.inc();
            PooledAudioFrame::new(frame, self.clone())
        }
    }

    /// Return a frame to the pool. Lock-free atomic stats; `push`
    /// returns the frame back on a full queue (matches the previous
    /// max-size guard — frame dropped, no stats updated).
    fn return_frame(&self, frame: ZeroCopyAudioFrame) {
        let matches_config = frame.sample_rate == self.config.sample_rate
            && frame.channels == self.config.channels
            && frame.buffer.len() == self.config.samples_per_frame * self.config.channels as usize;
        if !matches_config {
            return;
        }
        if self.pool.push(frame).is_ok() {
            self.returned_count.inc();
            let depth = self.pool.len();
            self.max_pool_size.update_max(depth);
        }
    }

    /// Get current pool statistics. Snapshots the atomic counters
    /// into the public `PoolStats` struct — fields are read with
    /// relaxed ordering and may show slight inter-field skew under
    /// concurrent writers (acceptable for telemetry).
    pub fn get_stats(&self) -> PoolStats {
        PoolStats {
            pool_size: self.pool.len(),
            allocated_count: self.allocated_count.get(),
            returned_count: self.returned_count.get(),
            pool_hits: self.pool_hits.get(),
            pool_misses: self.pool_misses.get(),
            max_pool_size: self.max_pool_size.get(),
            // RtpBufferPool-only fields stay zero on AudioFramePool.
            total_allocated: 0,
            available: 0,
            cache_hits: 0,
            cache_misses: 0,
            pool_exhausted: 0,
        }
    }

    /// Clear the pool and reset statistics
    pub fn clear(&self) {
        while self.pool.pop().is_some() {}
        self.pool_hits.set(0);
        self.pool_misses.set(0);
        self.allocated_count.set(0);
        self.returned_count.set(0);
        self.max_pool_size.set(0);
    }

    /// Pre-warm the pool with additional frames
    pub fn prewarm(&self, additional_frames: usize) {
        for _ in 0..additional_frames {
            let samples = vec![0i16; self.config.samples_per_frame * self.config.channels as usize];
            let frame =
                ZeroCopyAudioFrame::new(samples, self.config.sample_rate, self.config.channels, 0);
            if self.pool.push(frame).is_err() {
                break;
            }
        }
        self.max_pool_size.update_max(self.pool.len());
    }
}

/// Pool for RTP output buffers (for zero-copy encoding).
///
/// Same `ArrayQueue` + padded-atomic-counters split as
/// [`AudioFramePool`] — every per-packet `get_buffer`/return
/// previously took two `parking_lot::Mutex` guards on a shared
/// `PoolStats`, which became the next bottleneck after the metrics
/// RwLock was gated off in C17. Lock-free stats keep concurrent
/// scaling clean.
pub struct RtpBufferPool {
    buffers: ArrayQueue<Vec<u8>>,
    buffer_size: usize,
    initial_count: usize,
    max_count: usize,
    total_allocated: PaddedCounter,
    available: PaddedCounter,
    cache_hits: PaddedCounter,
    cache_misses: PaddedCounter,
    pool_exhausted: PaddedCounter,
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
        // ArrayQueue must be sized for the maximum number of buffers
        // we'll ever hold. `max_count` is the right upper bound.
        let queue = ArrayQueue::new(max_count.max(initial_count).max(1));

        // Pre-allocate initial buffers
        for _ in 0..initial_count {
            let mut buffer = Vec::with_capacity(buffer_size);
            buffer.resize(buffer_size, 0);
            // The queue has capacity ≥ initial_count, so push can't
            // fail here.
            let _ = queue.push(buffer);
        }

        debug!(
            "Created RTP buffer pool: size={}, initial={}, max={}",
            buffer_size, initial_count, max_count
        );

        let pool = Self {
            buffers: queue,
            buffer_size,
            initial_count,
            max_count,
            total_allocated: PaddedCounter::new(),
            available: PaddedCounter::new(),
            cache_hits: PaddedCounter::new(),
            cache_misses: PaddedCounter::new(),
            pool_exhausted: PaddedCounter::new(),
        };
        pool.total_allocated.set(initial_count);
        pool.available.set(initial_count);

        Arc::new(pool)
    }

    /// Get a buffer from the pool. Lock-free atomic stats: hit/miss
    /// counters increment independently on per-frame calls — no
    /// shared lock or cache line on the hot path.
    pub fn get_buffer(self: &Arc<Self>) -> PooledRtpBuffer {
        let buffer = if let Some(mut buffer) = self.buffers.pop() {
            buffer.clear();
            buffer.resize(self.buffer_size, 0);
            self.cache_hits.inc();
            self.available.dec();
            buffer
        } else {
            // Decide whether to allocate a new pool-resident buffer
            // or a temporary that won't rejoin the pool. The
            // total_allocated check is racy with concurrent get
            // calls, so we may temporarily over-allocate by O(threads)
            // — acceptable for a soft max-count budget.
            let mut buffer = Vec::with_capacity(self.buffer_size);
            buffer.resize(self.buffer_size, 0);
            if self.total_allocated.get() < self.max_count {
                self.total_allocated.inc();
                self.cache_misses.inc();
            } else {
                self.pool_exhausted.inc();
                warn!("RTP buffer pool exhausted, creating temporary buffer");
            }
            buffer
        };

        PooledRtpBuffer {
            buffer: Some(buffer),
            pool: self.clone(),
            capacity: self.buffer_size,
        }
    }

    /// Return a buffer to the pool. Lock-free atomic stats.
    fn return_buffer(&self, buffer: Vec<u8>) {
        if buffer.capacity() != self.buffer_size || self.buffers.len() >= self.initial_count {
            // Oversized or excess: drop and decrement bookkeeping.
            self.total_allocated.dec();
            return;
        }
        if self.buffers.push(buffer).is_ok() {
            self.available.inc();
        } else {
            // Race: queue filled between len() check and push. Drop.
            self.total_allocated.dec();
        }
    }

    /// Get pool statistics
    pub fn get_stats(&self) -> PoolStats {
        PoolStats {
            pool_size: self.buffers.len(),
            allocated_count: 0,
            returned_count: 0,
            pool_hits: 0,
            pool_misses: 0,
            max_pool_size: 0,
            total_allocated: self.total_allocated.get(),
            available: self.available.get(),
            cache_hits: self.cache_hits.get(),
            cache_misses: self.cache_misses.get(),
            pool_exhausted: self.pool_exhausted.get(),
        }
    }

    /// Reset pool statistics
    pub fn reset_stats(&self) {
        self.total_allocated.set(0);
        self.available.set(0);
        self.cache_hits.set(0);
        self.cache_misses.set(0);
        self.pool_exhausted.set(0);
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
        let _frame3 = pool.get_frame(); // Should trigger new allocation

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
