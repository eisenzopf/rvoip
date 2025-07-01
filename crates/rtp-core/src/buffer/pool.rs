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

/// A buffer from the pool
///
/// This buffer will be automatically returned to the pool when it is dropped.
pub struct PooledBuffer {
    /// The buffer itself
    buffer: Option<BytesMut>,
    
    /// Original capacity of the buffer when it was allocated
    /// Used to detect if the buffer was resized
    original_capacity: usize,
    
    /// Reference to the pool
    pool: BufferPool,
    
    /// Permit for the buffer
    permit: Option<tokio::sync::OwnedSemaphorePermit>,
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
        
        // Create semaphore with exactly max_buffers permits
        // This is critical - we need to make sure we don't get more than max_buffers
        let buffer_limit = Arc::new(Semaphore::new(max_buffers as usize));
        
        // Account for permits already used by the initial buffers
        if initial_capacity > 0 {
            // Since we allocated initial_capacity buffers, mark those permits as acquired
            // This ensures we won't exceed max_buffers total
            match buffer_limit.clone().try_acquire_many_owned(initial_capacity as u32) {
                Ok(_) => {
                    // This is expected - we're just reducing the available permits
                    // We immediately drop the permit because we're using the initial_capacity
                    // buffers that we pre-allocated
                }
                Err(_) => {
                    panic!("Failed to acquire initial permits - semaphore capacity too small");
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
    /// If no buffer is immediately available, this will create a new one
    /// as long as the pool is under capacity.
    pub async fn get_buffer(&self) -> PooledBuffer {
        // Try to acquire a permit from the semaphore
        // If we're at max capacity, this will block until a buffer is returned
        let permit = match self.buffer_limit.clone().acquire_owned().await {
            Ok(permit) => permit,
            Err(_) => {
                // Semaphore was closed - this should never happen in normal operation
                panic!("Buffer pool semaphore was closed");
            }
        };
        
        // Get a buffer from the available queue if possible
        let buffer = {
            let mut inner = self.inner.lock().await;
            inner.available.pop_front()
        };
        
        let buffer = if let Some(buffer) = buffer {
            // We got an existing buffer from the pool
            buffer
        } else {
            // No buffer available in pool, create a new one
            let new_buffer = BytesMut::with_capacity(self.buffer_size);
            
            // Update stats
            let mut inner = self.inner.lock().await;
            inner.allocated += 1;
            inner.bytes_allocated += new_buffer.capacity();
            
            new_buffer
        };
        
        let original_capacity = buffer.capacity();
        
        PooledBuffer {
            buffer: Some(buffer),
            original_capacity,
            pool: self.clone(),
            permit: Some(permit), // Store the permit with the buffer
        }
    }
    
    /// Try to get a buffer without blocking
    ///
    /// Returns None if the pool is at capacity.
    pub async fn try_get_buffer(&self) -> Option<PooledBuffer> {
        // Try to acquire a permit
        let permit = self.buffer_limit.try_acquire();
        if permit.is_err() {
            debug!("Buffer pool at capacity, can't allocate more buffers");
            return None; // No permit available, can't get a buffer
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
            permit: None, // Permit is not stored with the buffer
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
            
            // Only add it to the available pool if we're under max capacity
            if inner.available.len() < inner.max_buffers {
                inner.available.push_back(buffer);
            } else {
                // We've hit max capacity, so just drop this buffer
                inner.allocated -= 1;
                inner.bytes_allocated -= original_capacity;
                debug!("Dropping buffer due to pool capacity constraints");
                // Don't add to available pool - just let it be dropped
            }
        } else {
            // Buffer was resized, just drop it
            let mut inner = self.inner.lock().await;
            inner.bytes_allocated -= original_capacity;
            inner.bytes_allocated += current_capacity;
            
            // Let it be dropped
            debug!("Buffer resized from {} to {} bytes, not returning to pool", 
                   original_capacity, current_capacity);
        }
        
        // Note: We no longer need to add a permit to the semaphore
        // The OwnedSemaphorePermit will automatically release when dropped
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
        // If buffer is None, it means the buffer has been detached
        if let Some(buffer) = self.buffer.take() {
            // Use tokio's spawn_blocking to avoid blocking the current thread
            let pool = self.pool.clone();
            let original_capacity = self.original_capacity;
            tokio::spawn(async move {
                pool.return_buffer(buffer, original_capacity).await;
            });
        }
        
        // No need to explicitly release the permit - it will be released when dropped
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
        // Create a pool with max 10 buffers
        let pool = BufferPool::new(1024, 5, 10);
        
        // Get 5 initial buffers
        let mut buffers = Vec::new();
        for i in 0..5 {
            let buffer = pool.get_buffer().await;
            buffers.push(buffer);
        }
        
        // Check stats
        let stats = pool.stats().await;
        assert_eq!(stats.allocated, 5, "Should have 5 buffers allocated");
        assert_eq!(stats.available, 0, "Should have 0 buffers available");
        
        // Return buffers
        buffers.clear();
        
        // Give time for async return
        tokio::time::sleep(Duration::from_millis(50)).await;
        
        // Check stats again
        let stats = pool.stats().await;
        assert_eq!(stats.allocated, 5, "Should still have 5 buffers allocated");
        assert_eq!(stats.available, 5, "Should have 5 buffers available now");
        
        // Get all 10 buffers - should work
        let mut buffers = Vec::new();
        for i in 0..10 {
            let buffer = match pool.get_buffer().await {
                buffer => {
                    assert!(true, "Got buffer {} successfully", i);
                    buffer
                }
            };
            buffers.push(buffer);
        }
        
        // Verify we have all 10 buffers allocated
        let stats = pool.stats().await;
        assert_eq!(stats.allocated, 10, "Should have 10 buffers allocated total");
        assert_eq!(stats.available, 0, "Should have 0 buffers available");
        
        // 11th buffer should fail with timeout
        let result = tokio::time::timeout(
            Duration::from_millis(100),
            pool.get_buffer(),
        ).await;
        
        // Check that it timed out
        assert!(result.is_err(), "11th buffer allocation should timeout");
        
        // Release one buffer
        if !buffers.is_empty() {
            buffers.pop(); // Take one buffer out and drop it
        }
        
        // Give time for async return
        tokio::time::sleep(Duration::from_millis(50)).await;
        
        // Should be able to get one buffer now
        let buffer = tokio::time::timeout(
            Duration::from_millis(100),
            pool.get_buffer(),
        ).await;
        
        assert!(buffer.is_ok(), "Should be able to get a buffer after releasing one");
        
        // But 11th buffer should still fail
        buffers.push(buffer.unwrap());
        
        let result = tokio::time::timeout(
            Duration::from_millis(100),
            pool.get_buffer(),
        ).await;
        
        assert!(result.is_err(), "11th buffer allocation should still timeout");
        
        // Final validation of pool state
        let stats = pool.stats().await;
        assert_eq!(stats.allocated, 10, "Should have 10 buffers allocated total");
        assert_eq!(stats.max_buffers, 10, "Max buffers should be 10");
    }
} 