//! Memory pool for efficient buffer reuse
//!
//! This module provides a pooled allocator for packet buffers to minimize
//! allocations and improve performance under high loads.

use bytes::{Bytes, BytesMut, Buf, BufMut};
use tokio::sync::{Mutex, Semaphore};
use std::sync::Arc;
use std::collections::VecDeque;
use tracing::{debug, warn};

/// A pool of reusable byte buffers
///
/// This provides efficient allocation and deallocation of buffers
/// by reusing memory when possible. It helps reduce GC pressure
/// in high-throughput scenarios.
#[derive(Clone)]
pub struct BufferPool {
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

/// A buffer acquired from the pool
///
/// This will be returned to the pool when dropped
pub struct PooledBuffer {
    /// The buffer from the pool
    buffer: Option<BytesMut>,
    
    /// Original capacity of the buffer
    original_capacity: usize,
    
    /// Reference to the pool
    pool: BufferPool,
}

impl BufferPool {
    /// Create a new buffer pool
    ///
    /// # Arguments
    ///
    /// * `buffer_size` - Size of each buffer
    /// * `initial_capacity` - Initial number of buffers to allocate
    /// * `max_buffers` - Maximum number of buffers in the pool
    pub fn new(buffer_size: usize, initial_capacity: usize, max_buffers: usize) -> Self {
        let initial_capacity = initial_capacity.min(max_buffers);
        
        // Pre-allocate buffers
        let mut available = VecDeque::with_capacity(initial_capacity);
        let mut bytes_allocated = 0;
        
        for _ in 0..initial_capacity {
            let buffer = BytesMut::with_capacity(buffer_size);
            bytes_allocated += buffer.capacity();
            available.push_back(buffer);
        }
        
        let inner = BufferPoolInner {
            available,
            allocated: initial_capacity,
            bytes_allocated,
            max_buffers,
            buffer_size,
        };
        
        // Create semaphore with max_buffers permits
        let buffer_limit = Arc::new(Semaphore::new(max_buffers));
        
        // Acquire permits for initial buffers
        if initial_capacity > 0 {
            // Use try_acquire_many (non-owned version) to avoid consuming buffer_limit
            match buffer_limit.try_acquire_many(initial_capacity as u32) {
                Ok(permits) => {
                    // Just immediately drop the permits - we're just limiting initial capacity
                    drop(permits);
                },
                Err(_) => {
                    // Should never happen since we're creating a fresh semaphore
                    panic!("Failed to acquire initial permits");
                }
            }
        }
        
        Self {
            inner: Arc::new(Mutex::new(inner)),
            buffer_limit,
            buffer_size,
        }
    }
    
    /// Get a buffer from the pool
    ///
    /// This will block if the pool is at capacity.
    pub async fn get_buffer(&self) -> PooledBuffer {
        // Acquire a permit for this buffer
        let _permit = self.buffer_limit.acquire().await.unwrap();
        
        // Try to get a buffer from the available queue
        let mut buffer = {
            let mut inner = self.inner.lock().await;
            inner.available.pop_front()
        };
        
        // If no buffer is available, create a new one
        if buffer.is_none() {
            // Create a new buffer
            let new_buffer = BytesMut::with_capacity(self.buffer_size);
            
            // Update stats
            let mut inner = self.inner.lock().await;
            inner.allocated += 1;
            inner.bytes_allocated += new_buffer.capacity();
            
            buffer = Some(new_buffer);
        }
        
        // Unwrap is safe because we either got a buffer or created one
        let buffer = buffer.unwrap();
        let original_capacity = buffer.capacity();
        
        PooledBuffer {
            buffer: Some(buffer),
            original_capacity,
            pool: self.clone(),
        }
    }
    
    /// Try to get a buffer without blocking
    ///
    /// Returns None if the pool is at capacity.
    pub async fn try_get_buffer(&self) -> Option<PooledBuffer> {
        // Try to acquire a permit
        let permit = self.buffer_limit.try_acquire();
        if permit.is_err() {
            return None;
        }
        
        // Try to get a buffer from the available queue
        let mut buffer = {
            let mut inner = self.inner.lock().await;
            inner.available.pop_front()
        };
        
        // If no buffer is available, create a new one
        if buffer.is_none() {
            // Create a new buffer
            let new_buffer = BytesMut::with_capacity(self.buffer_size);
            
            // Update stats
            let mut inner = self.inner.lock().await;
            inner.allocated += 1;
            inner.bytes_allocated += new_buffer.capacity();
            
            buffer = Some(new_buffer);
        }
        
        // Unwrap is safe because we either got a buffer or created one
        let buffer = buffer.unwrap();
        let original_capacity = buffer.capacity();
        
        Some(PooledBuffer {
            buffer: Some(buffer),
            original_capacity,
            pool: self.clone(),
        })
    }
    
    /// Return a buffer to the pool
    async fn return_buffer(&self, mut buffer: BytesMut, original_capacity: usize) {
        // Only return the buffer if it hasn't been resized
        let current_capacity = buffer.capacity();
        
        if current_capacity == original_capacity {
            // Reset the buffer for reuse
            buffer.clear();
            
            // Return to the pool
            let mut inner = self.inner.lock().await;
            inner.available.push_back(buffer);
        } else {
            // Buffer was resized, just drop it
            let mut inner = self.inner.lock().await;
            inner.bytes_allocated -= original_capacity;
            inner.bytes_allocated += current_capacity;
            
            // Let it be dropped
            debug!("Buffer resized from {} to {} bytes, not returning to pool", 
                   original_capacity, current_capacity);
        }
        
        // Release the semaphore permit
        self.buffer_limit.add_permits(1);
    }
    
    /// Get current pool statistics
    pub async fn stats(&self) -> BufferPoolStats {
        let inner = self.inner.lock().await;
        BufferPoolStats {
            allocated: inner.allocated,
            available: inner.available.len(),
            bytes_allocated: inner.bytes_allocated,
            max_buffers: inner.max_buffers,
            buffer_size: inner.buffer_size,
        }
    }
}

/// Buffer pool statistics
#[derive(Debug, Clone)]
pub struct BufferPoolStats {
    /// Total number of buffers allocated
    pub allocated: usize,
    
    /// Number of buffers available in the pool
    pub available: usize,
    
    /// Total bytes allocated
    pub bytes_allocated: usize,
    
    /// Maximum allowed buffers
    pub max_buffers: usize,
    
    /// Buffer size
    pub buffer_size: usize,
}

impl PooledBuffer {
    /// Get a reference to the inner buffer
    pub fn buffer(&self) -> Option<&BytesMut> {
        self.buffer.as_ref()
    }
    
    /// Get a mutable reference to the inner buffer
    pub fn buffer_mut(&mut self) -> Option<&mut BytesMut> {
        self.buffer.as_mut()
    }
    
    /// Consume the buffer and return the inner BytesMut
    pub fn into_inner(mut self) -> BytesMut {
        self.buffer.take().unwrap()
    }
    
    /// Freeze the buffer into immutable Bytes
    ///
    /// This consumes the buffer and returns an immutable view that
    /// efficiently handles reference counting.
    pub fn freeze(mut self) -> Bytes {
        let buffer = self.buffer.take().unwrap();
        buffer.freeze()
    }
}

impl Drop for PooledBuffer {
    fn drop(&mut self) {
        // If buffer is still present, return it to the pool
        if let Some(buffer) = self.buffer.take() {
            // Use a blocking_task to return the buffer since we can't await in drop
            let pool = self.pool.clone();
            let original_capacity = self.original_capacity;
            
            tokio::task::spawn(async move {
                pool.return_buffer(buffer, original_capacity).await;
            });
        }
    }
}

/// Global shared buffer pools for common sizes
pub struct SharedPools {
    /// Small buffer pool (128 bytes)
    pub small: BufferPool,
    
    /// Medium buffer pool (1KB)
    pub medium: BufferPool,
    
    /// Large buffer pool (8KB)
    pub large: BufferPool,
    
    /// Extra large buffer pool (64KB)
    pub extra_large: BufferPool,
}

impl SharedPools {
    /// Create new shared buffer pools
    pub fn new(max_buffers: usize) -> Self {
        Self {
            small: BufferPool::new(128, 1000, max_buffers),
            medium: BufferPool::new(1024, 500, max_buffers / 2),
            large: BufferPool::new(8 * 1024, 100, max_buffers / 10),
            extra_large: BufferPool::new(64 * 1024, 10, max_buffers / 100),
        }
    }
    
    /// Get a buffer of appropriate size
    pub async fn get_buffer_for_size(&self, size: usize) -> PooledBuffer {
        if size <= 128 {
            self.small.get_buffer().await
        } else if size <= 1024 {
            self.medium.get_buffer().await
        } else if size <= 8 * 1024 {
            self.large.get_buffer().await
        } else {
            self.extra_large.get_buffer().await
        }
    }
    
    /// Try to get a buffer of appropriate size without blocking
    pub async fn try_get_buffer_for_size(&self, size: usize) -> Option<PooledBuffer> {
        if size <= 128 {
            self.small.try_get_buffer().await
        } else if size <= 1024 {
            self.medium.try_get_buffer().await
        } else if size <= 8 * 1024 {
            self.large.try_get_buffer().await
        } else {
            self.extra_large.try_get_buffer().await
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::timeout;
    use std::time::Duration;
    
    #[tokio::test]
    async fn test_buffer_pool() {
        let pool = BufferPool::new(1024, 5, 10);
        
        // Get 5 buffers
        let mut buffers = Vec::new();
        for _ in 0..5 {
            let buffer = pool.get_buffer().await;
            buffers.push(buffer);
        }
        
        // Check stats
        let stats = pool.stats().await;
        assert_eq!(stats.allocated, 5);
        assert_eq!(stats.available, 0);
        
        // Return buffers
        buffers.clear();
        
        // Give time for async return
        tokio::time::sleep(Duration::from_millis(50)).await;
        
        // Check stats again
        let stats = pool.stats().await;
        assert_eq!(stats.allocated, 5);
        assert_eq!(stats.available, 5);
        
        // Get 10 buffers (should work)
        let mut buffers = Vec::new();
        for _ in 0..10 {
            let buffer = pool.get_buffer().await;
            buffers.push(buffer);
        }
        
        // 11th buffer should block
        let result = timeout(
            Duration::from_millis(100),
            pool.get_buffer(),
        ).await;
        assert!(result.is_err()); // Timeout expected
    }
} 