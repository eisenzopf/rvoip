//! Ring Buffer
//!
//! This module provides a high-performance circular ring buffer
//! for efficient fixed-size buffering operations.

use std::sync::atomic::{AtomicUsize, Ordering};
use tokio::sync::RwLock;
use tracing::trace;

/// Ring buffer error types
#[derive(Debug, Clone)]
pub enum RingBufferError {
    /// Buffer is full
    BufferFull,
    /// Buffer is empty
    BufferEmpty,
    /// Invalid capacity (must be power of 2)
    InvalidCapacity,
    /// Index out of bounds
    IndexOutOfBounds,
}

impl std::fmt::Display for RingBufferError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RingBufferError::BufferFull => write!(f, "Ring buffer is full"),
            RingBufferError::BufferEmpty => write!(f, "Ring buffer is empty"),
            RingBufferError::InvalidCapacity => write!(f, "Ring buffer capacity must be a power of 2"),
            RingBufferError::IndexOutOfBounds => write!(f, "Ring buffer index out of bounds"),
        }
    }
}

impl std::error::Error for RingBufferError {}

/// A high-performance circular ring buffer
pub struct RingBuffer<T> {
    /// Internal storage
    buffer: RwLock<Vec<Option<T>>>,
    /// Buffer capacity (must be power of 2)
    capacity: usize,
    /// Capacity mask for fast modulo operations
    capacity_mask: usize,
    /// Write position
    write_pos: AtomicUsize,
    /// Read position
    read_pos: AtomicUsize,
}

impl<T> RingBuffer<T> {
    /// Create a new ring buffer with the specified capacity
    /// 
    /// Capacity must be a power of 2 for optimal performance
    pub fn new(capacity: usize) -> Result<Self, RingBufferError> {
        // Ensure capacity is power of 2
        if capacity == 0 || (capacity & (capacity - 1)) != 0 {
            return Err(RingBufferError::InvalidCapacity);
        }
        
        let mut buffer = Vec::with_capacity(capacity);
        buffer.resize_with(capacity, || None);
        
        Ok(Self {
            buffer: RwLock::new(buffer),
            capacity,
            capacity_mask: capacity - 1,
            write_pos: AtomicUsize::new(0),
            read_pos: AtomicUsize::new(0),
        })
    }
    
    /// Create a ring buffer with capacity rounded up to next power of 2
    pub fn with_capacity_rounded(capacity: usize) -> Self {
        let rounded_capacity = if capacity == 0 {
            1
        } else {
            capacity.next_power_of_two()
        };
        
        Self::new(rounded_capacity).expect("Rounded capacity should be valid")
    }
    
    /// Push an item to the buffer
    /// 
    /// Returns an error if the buffer is full
    pub async fn push(&self, item: T) -> Result<(), RingBufferError> {
        let write_pos = self.write_pos.load(Ordering::Acquire);
        let read_pos = self.read_pos.load(Ordering::Acquire);
        
        // Check if buffer is full
        let next_write_pos = (write_pos + 1) & self.capacity_mask;
        if next_write_pos == read_pos {
            return Err(RingBufferError::BufferFull);
        }
        
        // Write the item
        {
            let mut buffer = self.buffer.write().await;
            buffer[write_pos] = Some(item);
        }
        
        // Update write position
        self.write_pos.store(next_write_pos, Ordering::Release);
        
        trace!("Ring buffer push: write_pos={}", next_write_pos);
        Ok(())
    }
    
    /// Pop an item from the buffer
    /// 
    /// Returns an error if the buffer is empty
    pub async fn pop(&self) -> Result<T, RingBufferError> {
        let read_pos = self.read_pos.load(Ordering::Acquire);
        let write_pos = self.write_pos.load(Ordering::Acquire);
        
        // Check if buffer is empty
        if read_pos == write_pos {
            return Err(RingBufferError::BufferEmpty);
        }
        
        // Read the item
        let item = {
            let mut buffer = self.buffer.write().await;
            buffer[read_pos].take()
        };
        
        let item = item.ok_or(RingBufferError::BufferEmpty)?;
        
        // Update read position
        let next_read_pos = (read_pos + 1) & self.capacity_mask;
        self.read_pos.store(next_read_pos, Ordering::Release);
        
        trace!("Ring buffer pop: read_pos={}", next_read_pos);
        Ok(item)
    }
    
    /// Peek at the next item without removing it
    pub async fn peek(&self) -> Result<T, RingBufferError> 
    where 
        T: Clone,
    {
        let read_pos = self.read_pos.load(Ordering::Acquire);
        let write_pos = self.write_pos.load(Ordering::Acquire);
        
        // Check if buffer is empty
        if read_pos == write_pos {
            return Err(RingBufferError::BufferEmpty);
        }
        
        // Read the item without removing it
        let buffer = self.buffer.read().await;
        let item = buffer[read_pos].as_ref().ok_or(RingBufferError::BufferEmpty)?;
        
        Ok(item.clone())
    }
    
    /// Get the number of items currently in the buffer
    pub fn len(&self) -> usize {
        let write_pos = self.write_pos.load(Ordering::Acquire);
        let read_pos = self.read_pos.load(Ordering::Acquire);
        
        (write_pos.wrapping_sub(read_pos)) & self.capacity_mask
    }
    
    /// Check if the buffer is empty
    pub fn is_empty(&self) -> bool {
        let write_pos = self.write_pos.load(Ordering::Acquire);
        let read_pos = self.read_pos.load(Ordering::Acquire);
        
        write_pos == read_pos
    }
    
    /// Check if the buffer is full
    pub fn is_full(&self) -> bool {
        let write_pos = self.write_pos.load(Ordering::Acquire);
        let read_pos = self.read_pos.load(Ordering::Acquire);
        
        ((write_pos + 1) & self.capacity_mask) == read_pos
    }
    
    /// Get the capacity of the buffer
    pub fn capacity(&self) -> usize {
        self.capacity
    }
    
    /// Get remaining space in the buffer
    pub fn remaining_capacity(&self) -> usize {
        self.capacity - self.len() - 1 // -1 because we can't completely fill the buffer
    }
    
    /// Clear all items from the buffer
    pub async fn clear(&self) {
        {
            let mut buffer = self.buffer.write().await;
            for item in buffer.iter_mut() {
                *item = None;
            }
        }
        
        self.write_pos.store(0, Ordering::Release);
        self.read_pos.store(0, Ordering::Release);
        
        trace!("Ring buffer cleared");
    }
    
    /// Force push an item, overwriting oldest item if buffer is full
    pub async fn force_push(&self, item: T) -> Option<T> {
        let write_pos = self.write_pos.load(Ordering::Acquire);
        let read_pos = self.read_pos.load(Ordering::Acquire);
        
        let next_write_pos = (write_pos + 1) & self.capacity_mask;
        let mut overwritten_item = None;
        
        // If buffer is full, advance read position to make space
        if next_write_pos == read_pos {
            // Read the item that will be overwritten
            {
                let mut buffer = self.buffer.write().await;
                overwritten_item = buffer[read_pos].take();
            }
            
            // Advance read position
            let next_read_pos = (read_pos + 1) & self.capacity_mask;
            self.read_pos.store(next_read_pos, Ordering::Release);
        }
        
        // Write the new item
        {
            let mut buffer = self.buffer.write().await;
            buffer[write_pos] = Some(item);
        }
        
        // Update write position
        self.write_pos.store(next_write_pos, Ordering::Release);
        
        trace!("Ring buffer force_push: write_pos={}, overwritten={}", 
               next_write_pos, overwritten_item.is_some());
        
        overwritten_item
    }
    
    /// Try to pop multiple items at once
    pub async fn pop_many(&self, max_items: usize) -> Vec<T> {
        let mut items = Vec::with_capacity(max_items);
        
        for _ in 0..max_items {
            match self.pop().await {
                Ok(item) => items.push(item),
                Err(_) => break, // Buffer empty
            }
        }
        
        items
    }
    
    /// Get buffer utilization as a percentage (0.0 to 1.0)
    pub fn utilization(&self) -> f32 {
        let len = self.len();
        let max_capacity = self.capacity - 1; // -1 because we can't completely fill
        
        if max_capacity == 0 {
            0.0
        } else {
            len as f32 / max_capacity as f32
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_ring_buffer_creation() {
        let buffer: RingBuffer<i32> = RingBuffer::new(8).unwrap();
        assert_eq!(buffer.capacity(), 8);
        assert!(buffer.is_empty());
        assert!(!buffer.is_full());
        assert_eq!(buffer.len(), 0);
    }
    
    #[tokio::test]
    async fn test_invalid_capacity() {
        // Not power of 2
        assert!(RingBuffer::<i32>::new(7).is_err());
        assert!(RingBuffer::<i32>::new(0).is_err());
        
        // Valid powers of 2
        assert!(RingBuffer::<i32>::new(1).is_ok());
        assert!(RingBuffer::<i32>::new(2).is_ok());
        assert!(RingBuffer::<i32>::new(4).is_ok());
        assert!(RingBuffer::<i32>::new(8).is_ok());
    }
    
    #[tokio::test]
    async fn test_capacity_rounded() {
        let buffer: RingBuffer<i32> = RingBuffer::with_capacity_rounded(7);
        assert_eq!(buffer.capacity(), 8); // Rounded up to next power of 2
        
        let buffer: RingBuffer<i32> = RingBuffer::with_capacity_rounded(15);
        assert_eq!(buffer.capacity(), 16);
    }
    
    #[tokio::test]
    async fn test_push_and_pop() {
        let buffer: RingBuffer<i32> = RingBuffer::new(4).unwrap();
        
        // Push items (capacity is 4, but effective capacity is 3 due to circular buffer design)
        buffer.push(1).await.unwrap();
        buffer.push(2).await.unwrap();
        buffer.push(3).await.unwrap();
        
        assert_eq!(buffer.len(), 3);
        assert!(!buffer.is_empty());
        assert!(buffer.is_full()); // Ring buffer is full at capacity-1 items
        
        // Pop items
        assert_eq!(buffer.pop().await.unwrap(), 1);
        assert_eq!(buffer.pop().await.unwrap(), 2);
        assert_eq!(buffer.pop().await.unwrap(), 3);
        
        assert_eq!(buffer.len(), 0);
        assert!(buffer.is_empty());
    }
    
    #[tokio::test]
    async fn test_buffer_full() {
        let buffer: RingBuffer<i32> = RingBuffer::new(4).unwrap();
        
        // Fill buffer (capacity - 1 items max)
        buffer.push(1).await.unwrap();
        buffer.push(2).await.unwrap();
        buffer.push(3).await.unwrap();
        
        assert!(buffer.is_full());
        
        // Try to push one more - should fail
        assert!(matches!(buffer.push(4).await, Err(RingBufferError::BufferFull)));
    }
    
    #[tokio::test]
    async fn test_buffer_empty() {
        let buffer: RingBuffer<i32> = RingBuffer::new(4).unwrap();
        
        // Try to pop from empty buffer
        assert!(matches!(buffer.pop().await, Err(RingBufferError::BufferEmpty)));
        assert!(matches!(buffer.peek().await, Err(RingBufferError::BufferEmpty)));
    }
    
    #[tokio::test]
    async fn test_peek() {
        let buffer: RingBuffer<i32> = RingBuffer::new(4).unwrap();
        
        buffer.push(42).await.unwrap();
        buffer.push(84).await.unwrap();
        
        // Peek should return first item without removing it
        assert_eq!(buffer.peek().await.unwrap(), 42);
        assert_eq!(buffer.len(), 2);
        
        // Pop should return the same item
        assert_eq!(buffer.pop().await.unwrap(), 42);
        assert_eq!(buffer.len(), 1);
        
        // Next peek should return next item
        assert_eq!(buffer.peek().await.unwrap(), 84);
    }
    
    #[tokio::test]
    async fn test_force_push() {
        let buffer: RingBuffer<i32> = RingBuffer::new(4).unwrap();
        
        // Fill buffer
        buffer.push(1).await.unwrap();
        buffer.push(2).await.unwrap();
        buffer.push(3).await.unwrap();
        
        assert!(buffer.is_full());
        
        // Force push should overwrite oldest item
        let overwritten = buffer.force_push(4).await;
        assert_eq!(overwritten, Some(1));
        
        // Buffer should still be full but with different items
        assert!(buffer.is_full());
        assert_eq!(buffer.pop().await.unwrap(), 2);
        assert_eq!(buffer.pop().await.unwrap(), 3);
        assert_eq!(buffer.pop().await.unwrap(), 4);
    }
    
    #[tokio::test]
    async fn test_pop_many() {
        let buffer: RingBuffer<i32> = RingBuffer::new(8).unwrap();
        
        // Add several items
        for i in 1..=5 {
            buffer.push(i).await.unwrap();
        }
        
        // Pop 3 items
        let items = buffer.pop_many(3).await;
        assert_eq!(items, vec![1, 2, 3]);
        assert_eq!(buffer.len(), 2);
        
        // Pop more than available
        let items = buffer.pop_many(5).await;
        assert_eq!(items, vec![4, 5]); // Only 2 available
        assert!(buffer.is_empty());
    }
    
    #[tokio::test]
    async fn test_utilization() {
        let buffer: RingBuffer<i32> = RingBuffer::new(4).unwrap();
        
        assert_eq!(buffer.utilization(), 0.0);
        
        buffer.push(1).await.unwrap();
        assert!((buffer.utilization() - 1.0/3.0).abs() < 0.01); // 1 of 3 max items
        
        buffer.push(2).await.unwrap();
        buffer.push(3).await.unwrap();
        assert!((buffer.utilization() - 1.0).abs() < 0.01); // Full
    }
    
    #[tokio::test]
    async fn test_clear() {
        let buffer: RingBuffer<i32> = RingBuffer::new(4).unwrap();
        
        buffer.push(1).await.unwrap();
        buffer.push(2).await.unwrap();
        
        assert_eq!(buffer.len(), 2);
        
        buffer.clear().await;
        
        assert_eq!(buffer.len(), 0);
        assert!(buffer.is_empty());
        assert!(!buffer.is_full());
    }
} 