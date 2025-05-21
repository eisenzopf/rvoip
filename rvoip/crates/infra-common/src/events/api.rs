//! Core trait definitions for the unified event system API.
//!
//! This module defines the core interfaces that all event system implementations must satisfy.
//! These traits provide a common abstraction layer over different event system implementations
//! while allowing specialized optimizations for each implementation.

use std::sync::Arc;
use std::time::Duration;
use crate::events::types::{Event, EventResult, EventError};
use async_trait::async_trait;

/// Core trait representing an event system.
///
/// This trait defines the common interface that all event system implementations
/// must provide, regardless of their internal implementation details.
#[async_trait]
pub trait EventSystem: Send + Sync + Clone {
    /// Starts the event system.
    ///
    /// This method initializes any resources needed for event processing.
    /// The specific behavior depends on the implementation.
    async fn start(&self) -> EventResult<()>;
    
    /// Shuts down the event system.
    ///
    /// This method gracefully terminates event processing and releases resources.
    async fn shutdown(&self) -> EventResult<()>;
    
    /// Creates a publisher for events of type `E`.
    ///
    /// # Type Parameters
    ///
    /// * `E` - The event type to publish
    ///
    /// # Returns
    ///
    /// A boxed publisher that can publish events of type `E`
    fn create_publisher<E: Event + 'static>(&self) -> Box<dyn EventPublisher<E>>;
    
    /// Subscribes to events of type `E`.
    ///
    /// # Type Parameters
    ///
    /// * `E` - The event type to subscribe to
    ///
    /// # Returns
    ///
    /// A boxed subscriber that can receive events of type `E`, or an error if
    /// subscription fails
    async fn subscribe<E: Event + 'static>(&self) -> EventResult<Box<dyn EventSubscriber<E>>>;
}

/// Core trait for event publishers.
///
/// This trait defines the operations that all event publishers must support,
/// regardless of their internal implementation details.
#[async_trait]
pub trait EventPublisher<E: Event>: Send + Sync {
    /// Publishes a single event.
    ///
    /// # Arguments
    ///
    /// * `event` - The event to publish
    ///
    /// # Returns
    ///
    /// `Ok(())` if the event was published successfully, or an error if publication fails
    async fn publish(&self, event: E) -> EventResult<()>;
    
    /// Publishes a batch of events.
    ///
    /// This method may be optimized for batch operation in some implementations.
    ///
    /// # Arguments
    ///
    /// * `events` - A vector of events to publish
    ///
    /// # Returns
    ///
    /// `Ok(())` if all events were published successfully, or an error if any publication fails
    async fn publish_batch(&self, events: Vec<E>) -> EventResult<()>;
}

/// Core trait for event subscribers.
///
/// This trait defines the operations that all event subscribers must support,
/// regardless of their internal implementation details.
#[async_trait]
pub trait EventSubscriber<E: Event>: Send {
    /// Receives the next event.
    ///
    /// This method waits indefinitely until an event is available.
    ///
    /// # Returns
    ///
    /// The next event, or an error if receiving fails
    async fn receive(&mut self) -> EventResult<Arc<E>>;
    
    /// Receives the next event with a timeout.
    ///
    /// This method waits up to the specified duration for an event to become available.
    ///
    /// # Arguments
    ///
    /// * `timeout` - The maximum time to wait for an event
    ///
    /// # Returns
    ///
    /// The next event, or an error if receiving fails or the timeout expires
    async fn receive_timeout(&mut self, timeout: Duration) -> EventResult<Arc<E>>;
    
    /// Tries to receive an event without blocking.
    ///
    /// This method returns immediately with `None` if no event is available.
    ///
    /// # Returns
    ///
    /// `Some(event)` if an event was available, `None` if no event was available,
    /// or an error if receiving fails
    fn try_receive(&mut self) -> EventResult<Option<Arc<E>>>;
}

/// Feature flag to enable static event system implementation.
pub const FEATURE_STATIC_EVENT_SYSTEM: &str = "static_event_system";

/// Feature flag to enable zero-copy event system implementation.
pub const FEATURE_ZERO_COPY_EVENT_SYSTEM: &str = "zero_copy_event_system"; 