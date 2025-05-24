//! Adaptive Buffer
//!
//! This module provides an adaptive buffer that can dynamically resize
//! based on usage patterns and network conditions.

use std::collections::VecDeque;
use tokio::sync::RwLock;
use tracing::{debug, trace};

use crate::error::{Result, BufferError};

/// Configuration for adaptive buffer
#[derive(Debug, Clone)]
pub struct AdaptiveConfig {
    /// Initial capacity
    pub initial_capacity: usize,
    /// Minimum capacity
    pub min_capacity: usize,
    /// Maximum capacity
    pub max_capacity: usize,
    /// Growth factor when expanding
    pub growth_factor: f32,
    /// Shrink threshold (buffer usage ratio)
    pub shrink_threshold: f32,
    /// Expand threshold (buffer usage ratio)
    pub expand_threshold: f32,
}

impl Default for AdaptiveConfig {
    fn default() -> Self {
        Self {
            initial_capacity: 100,
            min_capacity: 10,
            max_capacity: 1000,
            growth_factor: 1.5,
            shrink_threshold: 0.25,
            expand_threshold: 0.8,
        }
    }
}

/// An adaptive buffer that resizes based on usage patterns
pub struct AdaptiveBuffer<T> {
    /// Configuration
    config: AdaptiveConfig,
    /// Internal buffer
    buffer: RwLock<VecDeque<T>>,
    /// Current capacity
    capacity: RwLock<usize>,
    /// Usage statistics
    stats: RwLock<AdaptiveStats>,
}

/// Adaptive buffer statistics
#[derive(Debug, Clone, Default)]
pub struct AdaptiveStats {
    /// Total items added
    pub items_added: u64,
    /// Total items removed
    pub items_removed: u64,
    /// Number of expansions
    pub expansions: u64,
    /// Number of shrinks
    pub shrinks: u64,
    /// Current size
    pub current_size: usize,
    /// Current capacity
    pub current_capacity: usize,
    /// Peak size reached
    pub peak_size: usize,
}

impl<T> AdaptiveBuffer<T> {
    /// Create a new adaptive buffer
    pub fn new(config: AdaptiveConfig) -> Self {
        let initial_capacity = config.initial_capacity;
        
        Self {
            config,
            buffer: RwLock::new(VecDeque::with_capacity(initial_capacity)),
            capacity: RwLock::new(initial_capacity),
            stats: RwLock::new(AdaptiveStats {
                current_capacity: initial_capacity,
                ..Default::default()
            }),
        }
    }
    
    /// Add an item to the buffer
    pub async fn push(&self, item: T) -> Result<()> {
        let should_expand = {
            let buffer = self.buffer.read().await;
            let capacity = *self.capacity.read().await;
            
            // Check if we need to expand
            let usage_ratio = buffer.len() as f32 / capacity as f32;
            usage_ratio >= self.config.expand_threshold
        };
        
        if should_expand {
            self.expand().await?;
        }
        
        // Add the item
        {
            let mut buffer = self.buffer.write().await;
            buffer.push_back(item);
        }
        
        // Update statistics
        {
            let mut stats = self.stats.write().await;
            stats.items_added += 1;
            
            let buffer = self.buffer.read().await;
            stats.current_size = buffer.len();
            if stats.current_size > stats.peak_size {
                stats.peak_size = stats.current_size;
            }
        }
        
        Ok(())
    }
    
    /// Remove an item from the buffer
    pub async fn pop(&self) -> Option<T> {
        let item = {
            let mut buffer = self.buffer.write().await;
            buffer.pop_front()
        };
        
        if item.is_some() {
            // Update statistics
            {
                let mut stats = self.stats.write().await;
                stats.items_removed += 1;
                stats.current_size = stats.current_size.saturating_sub(1);
            }
            
            // Check if we should shrink
            let should_shrink = {
                let buffer = self.buffer.read().await;
                let capacity = *self.capacity.read().await;
                
                let usage_ratio = buffer.len() as f32 / capacity as f32;
                usage_ratio <= self.config.shrink_threshold && capacity > self.config.min_capacity
            };
            
            if should_shrink {
                if let Err(e) = self.shrink().await {
                    debug!("Failed to shrink buffer: {}", e);
                }
            }
        }
        
        item
    }
    
    /// Peek at the front item without removing it
    pub async fn peek(&self) -> Option<T> 
    where 
        T: Clone,
    {
        let buffer = self.buffer.read().await;
        buffer.front().cloned()
    }
    
    /// Get the current size of the buffer
    pub async fn len(&self) -> usize {
        self.buffer.read().await.len()
    }
    
    /// Check if the buffer is empty
    pub async fn is_empty(&self) -> bool {
        self.buffer.read().await.is_empty()
    }
    
    /// Get the current capacity
    pub async fn capacity(&self) -> usize {
        *self.capacity.read().await
    }
    
    /// Clear all items from the buffer
    pub async fn clear(&self) {
        let mut buffer = self.buffer.write().await;
        buffer.clear();
        
        let mut stats = self.stats.write().await;
        stats.current_size = 0;
    }
    
    /// Expand the buffer capacity
    async fn expand(&self) -> Result<()> {
        let new_capacity = {
            let current_capacity = *self.capacity.read().await;
            let new_cap = ((current_capacity as f32) * self.config.growth_factor) as usize;
            new_cap.min(self.config.max_capacity)
        };
        
        let current_capacity = *self.capacity.read().await;
        if new_capacity > current_capacity {
            // Reserve additional space
            {
                let mut buffer = self.buffer.write().await;
                buffer.reserve(new_capacity - current_capacity);
            }
            
            {
                let mut capacity = self.capacity.write().await;
                *capacity = new_capacity;
            }
            
            {
                let mut stats = self.stats.write().await;
                stats.expansions += 1;
                stats.current_capacity = new_capacity;
            }
            
            trace!("Expanded adaptive buffer: {} -> {}", current_capacity, new_capacity);
        }
        
        Ok(())
    }
    
    /// Shrink the buffer capacity
    async fn shrink(&self) -> Result<()> {
        let new_capacity = {
            let current_capacity = *self.capacity.read().await;
            let new_cap = ((current_capacity as f32) / self.config.growth_factor) as usize;
            new_cap.max(self.config.min_capacity)
        };
        
        let current_capacity = *self.capacity.read().await;
        if new_capacity < current_capacity {
            // Note: VecDeque doesn't have a shrink_to method, so we recreate it
            {
                let mut buffer = self.buffer.write().await;
                let items: Vec<T> = buffer.drain(..).collect();
                *buffer = VecDeque::with_capacity(new_capacity);
                buffer.extend(items);
            }
            
            {
                let mut capacity = self.capacity.write().await;
                *capacity = new_capacity;
            }
            
            {
                let mut stats = self.stats.write().await;
                stats.shrinks += 1;
                stats.current_capacity = new_capacity;
            }
            
            trace!("Shrunk adaptive buffer: {} -> {}", current_capacity, new_capacity);
        }
        
        Ok(())
    }
    
    /// Get buffer statistics
    pub async fn get_statistics(&self) -> AdaptiveStats {
        let mut stats = self.stats.read().await.clone();
        stats.current_size = self.len().await;
        stats.current_capacity = self.capacity().await;
        stats
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[tokio::test]
    async fn test_adaptive_buffer_creation() {
        let config = AdaptiveConfig::default();
        let buffer: AdaptiveBuffer<i32> = AdaptiveBuffer::new(config);
        
        assert_eq!(buffer.len().await, 0);
        assert!(buffer.is_empty().await);
        assert_eq!(buffer.capacity().await, 100);
    }
    
    #[tokio::test]
    async fn test_push_and_pop() {
        let buffer: AdaptiveBuffer<i32> = AdaptiveBuffer::new(AdaptiveConfig::default());
        
        // Add items
        buffer.push(1).await.unwrap();
        buffer.push(2).await.unwrap();
        buffer.push(3).await.unwrap();
        
        assert_eq!(buffer.len().await, 3);
        
        // Remove items
        assert_eq!(buffer.pop().await, Some(1));
        assert_eq!(buffer.pop().await, Some(2));
        assert_eq!(buffer.pop().await, Some(3));
        assert_eq!(buffer.pop().await, None);
        
        assert!(buffer.is_empty().await);
    }
    
    #[tokio::test]
    async fn test_buffer_expansion() {
        let config = AdaptiveConfig {
            initial_capacity: 5,
            expand_threshold: 0.8,
            growth_factor: 2.0,
            ..Default::default()
        };
        let buffer: AdaptiveBuffer<i32> = AdaptiveBuffer::new(config);
        
        // Fill buffer to trigger expansion
        for i in 0..5 {
            buffer.push(i).await.unwrap();
        }
        
        let stats = buffer.get_statistics().await;
        assert!(stats.expansions > 0);
        assert!(buffer.capacity().await > 5);
    }
} 