use std::time::Duration;
use std::sync::Arc;
use std::fmt;

use crate::events::system::EventSystem;
use crate::events::bus::EventBusConfig;

/// Implementation type for the event system
#[derive(Clone, Debug, PartialEq)]
pub enum ImplementationType {
    /// Static fast path optimized for high performance
    StaticFastPath,
    
    /// Zero-copy event bus with more features
    ZeroCopy,
}

/// Backpressure strategy to use when buffers are full
#[derive(Clone)]
pub enum BackpressureStrategy {
    /// Block publisher until buffer space is available
    Block,
    
    /// Drop oldest events to make room for new ones
    DropOldest,
    
    /// Drop newest events (reject new publishes)
    DropNewest,
    
    /// Apply custom backpressure function
    Custom(Arc<dyn Fn() -> BackpressureAction + Send + Sync>),
}

// Implement Debug manually
impl fmt::Debug for BackpressureStrategy {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            BackpressureStrategy::Block => write!(f, "Block"),
            BackpressureStrategy::DropOldest => write!(f, "DropOldest"),
            BackpressureStrategy::DropNewest => write!(f, "DropNewest"),
            BackpressureStrategy::Custom(_) => write!(f, "Custom(<function>)"),
        }
    }
}

/// Action to take when backpressure is applied
#[derive(Clone, Debug)]
pub enum BackpressureAction {
    /// Block until space is available
    Block,
    
    /// Drop the event
    Drop,
    
    /// Apply backoff and retry
    Backoff(Duration),
}

/// Builder for configuring and creating an event system
///
/// This builder provides a consistent way to configure either the static fast path
/// or zero-copy event bus implementation with the same interface.
///
/// # Examples
///
/// ```rust,no_run
/// use infra_common::events::builder::{EventSystemBuilder, ImplementationType};
/// use std::time::Duration;
///
/// # async fn example() -> Result<(), Box<dyn std::error::Error>> {
/// // Create a static fast path event system with default settings
/// let static_system = EventSystemBuilder::new()
///     .implementation(ImplementationType::StaticFastPath)
///     .build();
///
/// // Create a zero-copy event bus with custom settings
/// let zero_copy_system = EventSystemBuilder::new()
///     .implementation(ImplementationType::ZeroCopy)
///     .channel_capacity(5_000)
///     .max_concurrent_dispatches(200)
///     .enable_priority(true)
///     .default_timeout(Some(Duration::from_millis(500)))
///     .batch_size(50)
///     .shard_count(4)
///     .build();
/// # Ok(())
/// # }
/// ```
#[derive(Clone)]
pub struct EventSystemBuilder {
    implementation_type: ImplementationType,
    channel_capacity: usize,
    max_concurrent_dispatches: usize,
    enable_priority: bool,
    default_timeout: Option<Duration>,
    batch_size: usize,
    shard_count: usize,
    global_buffer_size: Option<usize>,
    backpressure_strategy: BackpressureStrategy,
    enable_metrics: bool,
    metrics_reporting_interval: Duration,
}

impl EventSystemBuilder {
    /// Create a new builder with sensible defaults
    pub fn new() -> Self {
        Self {
            implementation_type: ImplementationType::ZeroCopy,
            channel_capacity: 10_000,
            max_concurrent_dispatches: 1000,
            enable_priority: true,
            default_timeout: Some(Duration::from_secs(1)),
            batch_size: 100,
            shard_count: 8,
            global_buffer_size: None,
            backpressure_strategy: BackpressureStrategy::Block,
            enable_metrics: false,
            metrics_reporting_interval: Duration::from_secs(5),
        }
    }
    
    /// Set the implementation type
    pub fn implementation(mut self, implementation_type: ImplementationType) -> Self {
        self.implementation_type = implementation_type;
        self
    }
    
    /// Set the channel capacity for event buffers
    pub fn channel_capacity(mut self, capacity: usize) -> Self {
        self.channel_capacity = capacity;
        self
    }
    
    /// Set the maximum concurrent dispatches (zero-copy only)
    pub fn max_concurrent_dispatches(mut self, max: usize) -> Self {
        self.max_concurrent_dispatches = max;
        self
    }
    
    /// Enable or disable priority-based dispatching (zero-copy only)
    pub fn enable_priority(mut self, enabled: bool) -> Self {
        self.enable_priority = enabled;
        self
    }
    
    /// Set the default timeout for receive operations
    pub fn default_timeout(mut self, timeout: Option<Duration>) -> Self {
        self.default_timeout = timeout;
        self
    }
    
    /// Set the batch size for batch processing
    pub fn batch_size(mut self, size: usize) -> Self {
        self.batch_size = size;
        self
    }
    
    /// Set the shard count for sharded dispatchers (zero-copy only)
    pub fn shard_count(mut self, count: usize) -> Self {
        self.shard_count = count;
        self
    }
    
    /// Configure global buffer size (None = unlimited, within memory constraints)
    pub fn global_buffer_size(mut self, size: Option<usize>) -> Self {
        self.global_buffer_size = size;
        self
    }
    
    /// Configure backpressure strategy
    pub fn backpressure_strategy(mut self, strategy: BackpressureStrategy) -> Self {
        self.backpressure_strategy = strategy;
        self
    }
    
    /// Enable or disable metrics collection
    pub fn enable_metrics(mut self, enabled: bool) -> Self {
        self.enable_metrics = enabled;
        self
    }
    
    /// Set metrics reporting interval
    pub fn metrics_reporting_interval(mut self, interval: Duration) -> Self {
        self.metrics_reporting_interval = interval;
        self
    }
    
    /// Build the event system with the configured settings
    pub fn build(self) -> EventSystem {
        match self.implementation_type {
            ImplementationType::StaticFastPath => {
                EventSystem::new_static_fast_path(self.channel_capacity)
            },
            ImplementationType::ZeroCopy => {
                let config = EventBusConfig {
                    broadcast_capacity: self.channel_capacity,
                    max_concurrent_dispatches: self.max_concurrent_dispatches,
                    enable_priority: self.enable_priority,
                    default_timeout: self.default_timeout.unwrap_or(Duration::from_secs(1)),
                    enable_zero_copy: true,
                    batch_size: self.batch_size,
                    shard_count: self.shard_count,
                };
                
                EventSystem::new_zero_copy(config)
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_builder_defaults() {
        let builder = EventSystemBuilder::new();
        assert_eq!(builder.implementation_type, ImplementationType::ZeroCopy);
        assert_eq!(builder.channel_capacity, 10_000);
        assert_eq!(builder.max_concurrent_dispatches, 1000);
        assert!(builder.enable_priority);
        assert_eq!(builder.default_timeout, Some(Duration::from_secs(1)));
        assert_eq!(builder.batch_size, 100);
        assert_eq!(builder.shard_count, 8);
        assert_eq!(builder.global_buffer_size, None);
        assert!(matches!(builder.backpressure_strategy, BackpressureStrategy::Block));
        assert!(!builder.enable_metrics);
        assert_eq!(builder.metrics_reporting_interval, Duration::from_secs(5));
    }
    
    #[test]
    fn test_builder_customization() {
        let builder = EventSystemBuilder::new()
            .implementation(ImplementationType::StaticFastPath)
            .channel_capacity(5_000)
            .max_concurrent_dispatches(500)
            .enable_priority(false)
            .default_timeout(Some(Duration::from_millis(500)))
            .batch_size(50)
            .shard_count(4)
            .global_buffer_size(Some(100_000))
            .enable_metrics(true)
            .metrics_reporting_interval(Duration::from_secs(10));
            
        assert_eq!(builder.implementation_type, ImplementationType::StaticFastPath);
        assert_eq!(builder.channel_capacity, 5_000);
        assert_eq!(builder.max_concurrent_dispatches, 500);
        assert!(!builder.enable_priority);
        assert_eq!(builder.default_timeout, Some(Duration::from_millis(500)));
        assert_eq!(builder.batch_size, 50);
        assert_eq!(builder.shard_count, 4);
        assert_eq!(builder.global_buffer_size, Some(100_000));
        assert!(builder.enable_metrics);
        assert_eq!(builder.metrics_reporting_interval, Duration::from_secs(10));
    }
} 