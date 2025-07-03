//! High-performance buffer management for RTP packets
//!
//! This module provides efficient buffer management for both receiving and transmitting
//! RTP packets, optimized for high-scale deployments with tens of thousands of connections.

mod pool;
pub mod jitter;
pub mod transmit;

pub use pool::*;
pub use jitter::*;
pub use transmit::*;

use std::sync::Arc;
use tokio::sync::Semaphore;

/// Default maximum packets to buffer per stream
pub const DEFAULT_MAX_BUFFERED_PACKETS: usize = 1000;

/// Buffer limits configuration
#[derive(Debug, Clone)]
pub struct BufferLimits {
    /// Maximum number of packets to buffer per stream
    pub max_packets_per_stream: usize,
    
    /// Maximum packet size in bytes
    pub max_packet_size: usize,
    
    /// Maximum total memory to use across all buffers (0 = unlimited)
    pub max_memory: usize,
}

impl Default for BufferLimits {
    fn default() -> Self {
        Self {
            max_packets_per_stream: DEFAULT_MAX_BUFFERED_PACKETS,
            max_packet_size: crate::DEFAULT_MAX_PACKET_SIZE,
            max_memory: 0, // Unlimited by default
        }
    }
}

/// Global buffer management to enforce system-wide limits
pub struct GlobalBufferManager {
    /// Semaphore to limit total memory usage
    memory_semaphore: Option<Arc<Semaphore>>,
    
    /// Limits configuration
    limits: BufferLimits,
}

impl GlobalBufferManager {
    /// Create a new global buffer manager with the specified limits
    pub fn new(limits: BufferLimits) -> Self {
        // Create semaphore if memory limit is specified
        let memory_semaphore = if limits.max_memory > 0 {
            // Calculate how many "permits" to issue based on memory limit
            // Each permit represents a fixed chunk of memory
            let chunk_size = 1024; // 1KB chunks
            let permits = limits.max_memory / chunk_size;
            Some(Arc::new(Semaphore::new(permits)))
        } else {
            None
        };
        
        Self {
            memory_semaphore,
            limits,
        }
    }
    
    /// Acquire memory for a packet
    pub async fn acquire_memory(&self, size: usize) -> Option<MemoryPermit> {
        if let Some(semaphore) = &self.memory_semaphore {
            // Calculate number of chunks needed
            let chunk_size = 1024; // 1KB chunks
            let chunks = (size + chunk_size - 1) / chunk_size; // Round up
            
            // Try to acquire permits using the owned version
            let sem_clone = semaphore.clone();
            match Arc::clone(&sem_clone).try_acquire_many_owned(chunks as u32) {
                Ok(permit) => Some(MemoryPermit {
                    permit: Some(permit),
                    chunks,
                }),
                Err(_) => None, // Failed to acquire (would block)
            }
        } else {
            // No memory limits, so no need for a permit
            Some(MemoryPermit {
                permit: None,
                chunks: 0,
            })
        }
    }
    
    /// Get the buffer limits
    pub fn get_limits(&self) -> &BufferLimits {
        &self.limits
    }
}

/// RAII guard for memory allocation
pub struct MemoryPermit {
    permit: Option<tokio::sync::OwnedSemaphorePermit>,
    chunks: usize,
}

// Permit is automatically dropped when it goes out of scope
impl Drop for MemoryPermit {
    fn drop(&mut self) {
        if let Some(permit) = self.permit.take() {
            // Release the permit
            drop(permit);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::time::timeout;
    use std::time::Duration;
    
    #[tokio::test]
    async fn test_memory_limits() {
        // Create a buffer manager with a small memory limit
        let limits = BufferLimits {
            max_memory: 10 * 1024, // 10KB limit
            ..Default::default()
        };
        let manager = GlobalBufferManager::new(limits);
        
        // Should be able to acquire 5KB
        let permit1 = manager.acquire_memory(5 * 1024).await;
        assert!(permit1.is_some(), "Should be able to acquire 5KB");
        
        // Should be able to acquire 5KB more
        let permit2 = manager.acquire_memory(5 * 1024).await;
        assert!(permit2.is_some(), "Should be able to acquire another 5KB");
        
        // This should fail as we've used all 10KB
        let permit3 = manager.acquire_memory(1024).await;
        assert!(permit3.is_none(), "Should not be able to acquire more memory when limit reached");
        
        // Drop the first permit
        drop(permit1);
        
        // Now we should be able to acquire 5KB again
        let permit3 = manager.acquire_memory(5 * 1024).await;
        assert!(permit3.is_some(), "Should be able to acquire 5KB after releasing memory");
        
        // But not 6KB
        let permit4 = manager.acquire_memory(6 * 1024).await;
        assert!(permit4.is_none(), "Should not be able to acquire 6KB when only 5KB is available");
    }
} 