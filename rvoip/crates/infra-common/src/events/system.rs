//! Unified event system API.
//!
//! This module provides a consistent interface for working with event buses
//! in the system, supporting both high-performance static event paths and
//! feature-rich zero-copy event bus implementations.
//!
//! The core components of this module are:
//! - [`EventSystem`]: The main interface for event system operations
//! - [`EventPublisher`]: Type-specific publisher for events
//! - [`EventSubscriber`]: Type-specific subscriber for events

use std::sync::Arc;
use std::time::Duration;
use async_trait::async_trait;

use crate::events::types::{Event, EventResult};
use crate::events::bus::{EventBus, EventBusConfig};
use crate::events::api;
use crate::events::static_path::StaticFastPathSystem;
use crate::events::zero_copy::ZeroCopySystem;

/// Unified event system that provides a common interface to both implementations.
///
/// This struct abstracts over the underlying event system implementation,
/// allowing code to work with either implementation without changes.
#[derive(Clone)]
pub enum EventSystem {
    /// Static Fast Path implementation optimized for performance
    StaticFastPath(StaticFastPathSystem),
    
    /// Zero Copy implementation with advanced features
    ZeroCopy(ZeroCopySystem),
}

impl EventSystem {
    /// Creates a new event system using the static fast path implementation.
    ///
    /// This implementation provides maximum performance with minimal overhead,
    /// but lacks some of the advanced routing features of the zero-copy event bus.
    ///
    /// # Arguments
    ///
    /// * `channel_capacity` - The capacity of event channels
    ///
    /// # Returns
    ///
    /// A new `EventSystem` instance using the static fast path implementation
    pub fn new_static_fast_path(channel_capacity: usize) -> Self {
        Self::StaticFastPath(StaticFastPathSystem::new(channel_capacity))
    }
    
    /// Creates a new event system using the zero-copy event bus implementation.
    ///
    /// This implementation provides more features like priority-based routing,
    /// timeouts, and other advanced capabilities at the cost of some performance.
    ///
    /// # Arguments
    ///
    /// * `config` - The configuration for the event bus
    ///
    /// # Returns
    ///
    /// A new `EventSystem` instance using the zero-copy event bus implementation
    pub fn new_zero_copy(config: EventBusConfig) -> Self {
        Self::ZeroCopy(ZeroCopySystem::new(config))
    }
    
    /// Access the underlying zero-copy event bus for advanced operations.
    ///
    /// This method provides access to the underlying `EventBus` instance
    /// when using the zero-copy implementation, allowing advanced operations
    /// that aren't available through the unified API.
    ///
    /// # Returns
    ///
    /// Some reference to the `EventBus` if using zero-copy implementation,
    /// or None if using static fast path
    pub fn advanced(&self) -> Option<&EventBus> {
        match self {
            Self::StaticFastPath(_) => None,
            Self::ZeroCopy(system) => Some(system.event_bus()),
        }
    }
}

#[async_trait]
impl api::EventSystem for EventSystem {
    async fn start(&self) -> EventResult<()> {
        match self {
            Self::StaticFastPath(system) => system.start().await,
            Self::ZeroCopy(system) => system.start().await,
        }
    }
    
    async fn shutdown(&self) -> EventResult<()> {
        match self {
            Self::StaticFastPath(system) => system.shutdown().await,
            Self::ZeroCopy(system) => system.shutdown().await,
        }
    }
    
    fn create_publisher<E: Event + 'static>(&self) -> Box<dyn api::EventPublisher<E>> {
        match self {
            Self::StaticFastPath(system) => system.create_publisher::<E>(),
            Self::ZeroCopy(system) => system.create_publisher::<E>(),
        }
    }
    
    async fn subscribe<E: Event + 'static>(&self) -> EventResult<Box<dyn api::EventSubscriber<E>>> {
        match self {
            Self::StaticFastPath(system) => system.subscribe::<E>().await,
            Self::ZeroCopy(system) => system.subscribe::<E>().await,
        }
    }
}

/// Public wrapper for EventPublisher with a concrete type.
///
/// This struct provides a more convenient interface than using trait objects directly.
pub struct EventPublisher<E: Event> {
    /// The underlying publisher implementation
    inner: Box<dyn api::EventPublisher<E>>,
}

impl<E: Event + 'static> EventPublisher<E> {
    /// Creates a new EventPublisher from a boxed trait object.
    ///
    /// # Arguments
    ///
    /// * `inner` - The boxed trait object implementing the publisher
    ///
    /// # Returns
    ///
    /// A new `EventPublisher<E>` instance
    pub fn new(inner: Box<dyn api::EventPublisher<E>>) -> Self {
        Self { inner }
    }
    
    /// Publishes a single event.
    ///
    /// # Arguments
    ///
    /// * `event` - The event to publish
    ///
    /// # Returns
    ///
    /// `Ok(())` if the event was published successfully, or an error if publication fails
    pub async fn publish(&self, event: E) -> EventResult<()> {
        self.inner.publish(event).await
    }
    
    /// Publishes a batch of events.
    ///
    /// # Arguments
    ///
    /// * `events` - The events to publish
    ///
    /// # Returns
    ///
    /// `Ok(())` if all events were published successfully, or an error if any publication fails
    pub async fn publish_batch(&self, events: Vec<E>) -> EventResult<()> {
        self.inner.publish_batch(events).await
    }
}

/// Public wrapper for EventSubscriber with a concrete type.
///
/// This struct provides a more convenient interface than using trait objects directly.
pub struct EventSubscriber<E: Event> {
    /// The underlying subscriber implementation
    inner: Box<dyn api::EventSubscriber<E>>,
}

impl<E: Event + 'static> EventSubscriber<E> {
    /// Creates a new EventSubscriber from a boxed trait object.
    ///
    /// # Arguments
    ///
    /// * `inner` - The boxed trait object implementing the subscriber
    ///
    /// # Returns
    ///
    /// A new `EventSubscriber<E>` instance
    pub fn new(inner: Box<dyn api::EventSubscriber<E>>) -> Self {
        Self { inner }
    }
    
    /// Receives the next event.
    ///
    /// This method will wait indefinitely until an event is available.
    ///
    /// # Returns
    ///
    /// The next event, or an error if receiving fails
    pub async fn receive(&mut self) -> EventResult<Arc<E>> {
        self.inner.receive().await
    }
    
    /// Receives the next event with a timeout.
    ///
    /// # Arguments
    ///
    /// * `timeout` - The maximum time to wait for an event
    ///
    /// # Returns
    ///
    /// The next event, or an error if receiving fails or the timeout expires
    pub async fn receive_timeout(&mut self, timeout: Duration) -> EventResult<Arc<E>> {
        self.inner.receive_timeout(timeout).await
    }
    
    /// Tries to receive an event without blocking.
    ///
    /// # Returns
    ///
    /// `Some(event)` if an event was available, `None` if no event was available,
    /// or an error if receiving fails
    pub fn try_receive(&mut self) -> EventResult<Option<Arc<E>>> {
        self.inner.try_receive()
    }
} 