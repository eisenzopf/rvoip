//! Memory pool for efficient media buffer reuse (moved from rtp-core)
//!
//! This module provides a pooled allocator for media packet buffers to minimize
//! allocations and improve performance under high loads.

use bytes::{Bytes, BytesMut, Buf, BufMut};
use tokio::sync::{Mutex, Semaphore};
use std::sync::Arc;
use std::collections::VecDeque;
use tracing::{debug, warn};

/// A pool of reusable byte buffers for media processing
///
/// This provides efficient allocation and deallocation of buffers
/// by reusing memory when possible. It helps reduce GC pressure
/// in high-throughput media scenarios.
#[derive(Clone)]
pub struct MediaBufferPool {
    /// Inner state protected by a mutex
    inner: Arc<Mutex<BufferPoolInner>>,
    
    /// Semaphore to limit total number of buffers
    buffer_limit: Arc<Semaphore>,
    
    /// Buffer size
    buffer_size: usize,
}

/// Inner state of the buffer pool
struct BufferPoolInner {
    /// Available buffers
    available: VecDeque<BytesMut>,
    
    /// Total buffers allocated
    allocated: usize,
    
    /// Total bytes allocated
    bytes_allocated: usize,
    
    /// Maximum allowed buffers
    max_buffers: usize,
    
    /// Buffer size
    buffer_size: usize,
}

/// A buffer from the media pool
///
/// This buffer will be automatically returned to the pool when it is dropped.
pub struct PooledMediaBuffer {
    /// The buffer data
    buffer: Option<BytesMut>,
    
    /// Reference to the pool for returning the buffer
    pool: MediaBufferPool,
}

/// Statistics about buffer pool usage
#[derive(Debug, Clone)]
pub struct MediaBufferPoolStats {
    /// Number of buffers currently allocated
    pub allocated_buffers: usize,
    
    /// Total bytes allocated
    pub bytes_allocated: usize,
    
    /// Number of buffers available for reuse
    pub available_buffers: usize,
    
    /// Maximum buffers allowed
    pub max_buffers: usize,
    
    /// Buffer size
    pub buffer_size: usize,
    
    /// Pool utilization as percentage
    pub utilization: f64,
}

impl MediaBufferPool {
    /// Create a new buffer pool
    ///
    /// # Arguments
    /// * `buffer_size` - Size of each buffer in bytes
    /// * `max_buffers` - Maximum number of buffers to maintain
    pub fn new(buffer_size: usize, max_buffers: usize) -> Self {
        Self {
            inner: Arc::new(Mutex::new(BufferPoolInner {
                available: VecDeque::new(),
                allocated: 0,
                bytes_allocated: 0,
                max_buffers,
                buffer_size,
            })),
            buffer_limit: Arc::new(Semaphore::new(max_buffers)),
            buffer_size,
        }
    }
    
    /// Get a buffer from the pool
    ///
    /// This will reuse an existing buffer if available, or allocate a new one
    /// if under the limit.
    pub async fn get_buffer(&self) -> Result<PooledMediaBuffer, String> {
        // Acquire permit from semaphore
        let _permit = self.buffer_limit.acquire().await
            .map_err(|e| format!("Failed to acquire buffer permit: {}", e))?;
        
        let mut inner = self.inner.lock().await;
        
        let buffer = if let Some(mut reused_buffer) = inner.available.pop_front() {
            // Reuse existing buffer
            reused_buffer.clear();
            reused_buffer.resize(self.buffer_size, 0);
            debug!("Reusing buffer from pool, {} available", inner.available.len());
            reused_buffer
        } else {
            // Allocate new buffer
            if inner.allocated >= inner.max_buffers {
                warn!("Buffer pool exhausted, cannot allocate more buffers");
                return Err("Buffer pool exhausted".to_string());
            }
            
            let new_buffer = BytesMut::zeroed(self.buffer_size);
            inner.allocated += 1;
            inner.bytes_allocated += self.buffer_size;
            
            debug!("Allocated new buffer, total: {}", inner.allocated);
            new_buffer
        };
        
        Ok(PooledMediaBuffer {
            buffer: Some(buffer),
            pool: self.clone(),
        })
    }
    
    /// Return a buffer to the pool
    async fn return_buffer(&self, buffer: BytesMut) {
        let mut inner = self.inner.lock().await;
        
        // Only keep the buffer if we're under the max limit
        if inner.available.len() < inner.max_buffers / 2 {
            inner.available.push_back(buffer);
            debug!("Returned buffer to pool, {} available", inner.available.len());
        } else {
            // Pool is full, just drop the buffer
            debug!("Pool full, dropping returned buffer");
        }
    }
    
    /// Get statistics about the buffer pool
    pub async fn stats(&self) -> MediaBufferPoolStats {
        let inner = self.inner.lock().await;
        
        MediaBufferPoolStats {
            allocated_buffers: inner.allocated,
            bytes_allocated: inner.bytes_allocated,
            available_buffers: inner.available.len(),
            max_buffers: inner.max_buffers,
            buffer_size: inner.buffer_size,
            utilization: (inner.allocated as f64 / inner.max_buffers as f64) * 100.0,
        }
    }
    
    /// Clear all buffers from the pool
    pub async fn clear(&self) {
        let mut inner = self.inner.lock().await;
        inner.available.clear();
        inner.allocated = 0;
        inner.bytes_allocated = 0;
        debug!("Cleared buffer pool");
    }
}

impl PooledMediaBuffer {
    /// Get a mutable reference to the buffer data
    pub fn as_mut(&mut self) -> &mut BytesMut {
        self.buffer.as_mut().expect("Buffer should be present")
    }
    
    /// Get a reference to the buffer data
    pub fn as_ref(&self) -> &BytesMut {
        self.buffer.as_ref().expect("Buffer should be present")
    }
    
    /// Convert the buffer to Bytes (consumes the buffer)
    pub fn freeze(mut self) -> Bytes {
        let buffer = self.buffer.take().expect("Buffer should be present");
        buffer.freeze()
    }
    
    /// Get the size of the buffer
    pub fn len(&self) -> usize {
        self.buffer.as_ref().map_or(0, |b| b.len())
    }
    
    /// Check if the buffer is empty
    pub fn is_empty(&self) -> bool {
        self.len() == 0
    }
}

impl Drop for PooledMediaBuffer {
    fn drop(&mut self) {
        if let Some(buffer) = self.buffer.take() {
            // Return buffer to pool asynchronously
            let pool = self.pool.clone();
            tokio::spawn(async move {
                pool.return_buffer(buffer).await;
            });
        }
    }
}

impl std::ops::Deref for PooledMediaBuffer {
    type Target = BytesMut;
    
    fn deref(&self) -> &Self::Target {
        self.as_ref()
    }
}

impl std::ops::DerefMut for PooledMediaBuffer {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.as_mut()
    }
}

/// Create a default media buffer pool suitable for typical media processing
pub fn create_default_media_pool() -> MediaBufferPool {
    // Default: 1500 byte buffers (typical MTU), max 1000 buffers
    MediaBufferPool::new(1500, 1000)
}

/// Create a large media buffer pool for high-throughput scenarios
pub fn create_large_media_pool() -> MediaBufferPool {
    // Large: 9000 byte buffers (jumbo frames), max 2000 buffers
    MediaBufferPool::new(9000, 2000)
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_buffer_pool_basic() {
        let pool = MediaBufferPool::new(1024, 10);
        
        // Get a buffer
        let buffer = pool.get_buffer().await.unwrap();
        assert_eq!(buffer.len(), 1024);
        
        // Check stats
        let stats = pool.stats().await;
        assert_eq!(stats.allocated_buffers, 1);
        assert_eq!(stats.buffer_size, 1024);
    }
    
    #[tokio::test]
    async fn test_buffer_pool_reuse() {
        let pool = MediaBufferPool::new(512, 5);
        
        // Get and return a buffer
        {
            let _buffer = pool.get_buffer().await.unwrap();
        } // Buffer is returned here via Drop
        
        tokio::task::yield_now().await; // Allow drop to complete
        
        // Get another buffer - should reuse
        let buffer2 = pool.get_buffer().await.unwrap();
        assert_eq!(buffer2.len(), 512);
        
        let stats = pool.stats().await;
        assert_eq!(stats.allocated_buffers, 1); // Should still be 1 due to reuse
    }
    
    #[tokio::test]
    async fn test_buffer_pool_limit() {
        let pool = MediaBufferPool::new(256, 2);
        
        // Get maximum buffers
        let _buffer1 = pool.get_buffer().await.unwrap();
        let _buffer2 = pool.get_buffer().await.unwrap();
        
        // This should fail
        let result = pool.get_buffer().await;
        assert!(result.is_err());
    }
}