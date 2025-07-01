//! Builder pattern for event systems.
//!
//! This module provides a builder pattern for creating event systems with
//! specific configurations, allowing users to customize the event system
//! to their needs without dealing with implementation details.

use std::time::Duration;
use crate::events::bus::EventBusConfig;
use super::system::EventSystem;

/// Enum representing the implementation type to use.
///
/// This enum allows users to select which implementation of the event system
/// to use, without having to know the details of each implementation.
#[derive(Debug, Clone, Copy)]
pub enum ImplementationType {
    /// Use the Static Fast Path implementation for maximum performance
    StaticFastPath,
    
    /// Use the Zero Copy implementation for advanced features
    ZeroCopy,
}

/// Builder for creating event systems with specific configurations.
///
/// This struct allows configuring various aspects of the event system before
/// creating it, providing sensible defaults for most options.
#[derive(Debug, Clone)]
pub struct EventSystemBuilder {
    /// The implementation type to use
    implementation: ImplementationType,
    
    /// The capacity of event channels
    channel_capacity: usize,
    
    /// The maximum number of concurrent dispatches (zero-copy only)
    max_concurrent_dispatches: usize,
    
    /// Whether to enable priority-based routing (zero-copy only)
    enable_priority: bool,
    
    /// The default timeout for operations (zero-copy only)
    default_timeout: Option<Duration>,
    
    /// The batch size for batch operations (zero-copy only)
    batch_size: usize,
    
    /// The number of shards to use for the event bus (zero-copy only)
    shard_count: usize,
    
    /// Whether to enable metrics collection (zero-copy only)
    enable_metrics: bool,
    
    /// The interval for reporting metrics (zero-copy only)
    metrics_reporting_interval: Duration,
}

impl Default for EventSystemBuilder {
    fn default() -> Self {
        Self {
            implementation: ImplementationType::ZeroCopy,
            channel_capacity: 10_000,
            max_concurrent_dispatches: 1_000,
            enable_priority: true,
            default_timeout: Some(Duration::from_secs(1)),
            batch_size: 100,
            shard_count: 8,
            enable_metrics: false,
            metrics_reporting_interval: Duration::from_secs(5),
        }
    }
}

impl EventSystemBuilder {
    /// Creates a new builder with default values.
    ///
    /// # Returns
    ///
    /// A new `EventSystemBuilder` instance with default values
    pub fn new() -> Self {
        Self::default()
    }
    
    /// Sets the implementation type to use.
    ///
    /// # Arguments
    ///
    /// * `implementation` - The implementation type to use
    ///
    /// # Returns
    ///
    /// `self` for method chaining
    pub fn implementation(mut self, implementation: ImplementationType) -> Self {
        self.implementation = implementation;
        self
    }
    
    /// Sets the channel capacity.
    ///
    /// This option applies to both implementations.
    ///
    /// # Arguments
    ///
    /// * `capacity` - The capacity of event channels
    ///
    /// # Returns
    ///
    /// `self` for method chaining
    pub fn channel_capacity(mut self, capacity: usize) -> Self {
        self.channel_capacity = capacity;
        self
    }
    
    /// Sets the maximum number of concurrent dispatches.
    ///
    /// This option only applies to the Zero Copy implementation.
    ///
    /// # Arguments
    ///
    /// * `max_dispatches` - The maximum number of concurrent dispatches
    ///
    /// # Returns
    ///
    /// `self` for method chaining
    pub fn max_concurrent_dispatches(mut self, max_dispatches: usize) -> Self {
        self.max_concurrent_dispatches = max_dispatches;
        self
    }
    
    /// Sets whether to enable priority-based routing.
    ///
    /// This option only applies to the Zero Copy implementation.
    ///
    /// # Arguments
    ///
    /// * `enable` - Whether to enable priority-based routing
    ///
    /// # Returns
    ///
    /// `self` for method chaining
    pub fn enable_priority(mut self, enable: bool) -> Self {
        self.enable_priority = enable;
        self
    }
    
    /// Sets the default timeout for operations.
    ///
    /// This option only applies to the Zero Copy implementation.
    ///
    /// # Arguments
    ///
    /// * `timeout` - The default timeout for operations
    ///
    /// # Returns
    ///
    /// `self` for method chaining
    pub fn default_timeout(mut self, timeout: Option<Duration>) -> Self {
        self.default_timeout = timeout;
        self
    }
    
    /// Sets the batch size for batch operations.
    ///
    /// This option only applies to the Zero Copy implementation.
    ///
    /// # Arguments
    ///
    /// * `size` - The batch size for batch operations
    ///
    /// # Returns
    ///
    /// `self` for method chaining
    pub fn batch_size(mut self, size: usize) -> Self {
        self.batch_size = size;
        self
    }
    
    /// Sets the number of shards to use for the event bus.
    ///
    /// This option only applies to the Zero Copy implementation.
    ///
    /// # Arguments
    ///
    /// * `count` - The number of shards to use
    ///
    /// # Returns
    ///
    /// `self` for method chaining
    pub fn shard_count(mut self, count: usize) -> Self {
        self.shard_count = count;
        self
    }
    
    /// Sets whether to enable metrics collection.
    ///
    /// This option only applies to the Zero Copy implementation.
    ///
    /// # Arguments
    ///
    /// * `enable` - Whether to enable metrics collection
    ///
    /// # Returns
    ///
    /// `self` for method chaining
    pub fn enable_metrics(mut self, enable: bool) -> Self {
        self.enable_metrics = enable;
        self
    }
    
    /// Sets the interval for reporting metrics.
    ///
    /// This option only applies to the Zero Copy implementation.
    ///
    /// # Arguments
    ///
    /// * `interval` - The interval for reporting metrics
    ///
    /// # Returns
    ///
    /// `self` for method chaining
    pub fn metrics_reporting_interval(mut self, interval: Duration) -> Self {
        self.metrics_reporting_interval = interval;
        self
    }
    
    /// Builds an event system with the configured options.
    ///
    /// # Returns
    ///
    /// A new `EventSystem` instance with the configured options
    pub fn build(self) -> EventSystem {
        match self.implementation {
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