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

use crate::events::types::{Event, EventResult, EventError, EventFilter, StaticEvent};
use crate::events::bus::{EventBus, EventBusConfig};
use crate::events::api::{self, EventSystem as EventSystemTrait};
use crate::events::static_path::StaticFastPathSystem;
use crate::events::zero_copy::ZeroCopySystem;
use crate::events::builder::{ImplementationType, EventSystemBuilder};

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

    /// Subscribe to events with a filter.
    ///
    /// This is a convenience method that combines subscribing and filtering in one step.
    ///
    /// # Type Parameters
    ///
    /// * `E` - The event type to subscribe to
    ///
    /// # Arguments
    ///
    /// * `filter` - A function that takes a reference to an event and returns a boolean indicating whether to accept it
    ///
    /// # Returns
    ///
    /// A subscriber that will only receive events that pass the filter
    pub async fn subscribe_filtered<E, F>(&self, filter: F) -> EventResult<Box<dyn api::EventSubscriber<E>>>
    where
        E: Event + 'static,
        F: Fn(&E) -> bool + Send + Sync + 'static,
    {
        match self {
            Self::StaticFastPath(system) => system.subscribe_filtered(filter).await,
            Self::ZeroCopy(system) => system.subscribe_filtered(filter).await,
        }
    }
    
    /// Subscribe to events with a predefined filter.
    ///
    /// This method is similar to `subscribe_filtered`, but takes an `EventFilter` instead of a function.
    ///
    /// # Type Parameters
    ///
    /// * `E` - The event type to subscribe to
    ///
    /// # Arguments
    ///
    /// * `filter` - An `EventFilter` for the event type
    ///
    /// # Returns
    ///
    /// A subscriber that will only receive events that pass the filter
    pub async fn subscribe_with_filter<E: Event + 'static>(&self, filter: EventFilter<E>) -> EventResult<Box<dyn api::EventSubscriber<E>>> {
        match self {
            Self::StaticFastPath(system) => system.subscribe_with_filter(filter).await,
            Self::ZeroCopy(system) => system.subscribe_with_filter(filter).await,
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
    
    async fn subscribe_filtered<E, F>(&self, filter: F) -> EventResult<Box<dyn api::EventSubscriber<E>>> 
    where
        E: Event + 'static,
        F: Fn(&E) -> bool + Send + Sync + 'static,
    {
        match self {
            Self::StaticFastPath(system) => system.subscribe_filtered(filter).await,
            Self::ZeroCopy(system) => system.subscribe_filtered(filter).await,
        }
    }
    
    async fn subscribe_with_filter<E: Event + 'static>(&self, filter: EventFilter<E>) -> EventResult<Box<dyn api::EventSubscriber<E>>> {
        match self {
            Self::StaticFastPath(system) => system.subscribe_with_filter(filter).await,
            Self::ZeroCopy(system) => system.subscribe_with_filter(filter).await,
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::types::EventPriority;
    use crate::events::builder::{EventSystemBuilder, ImplementationType};
    use crate::events::api::EventSystem as EventSystemTrait;
    use serde::{Serialize, Deserialize};
    use std::any::Any;
    use std::sync::Arc;
    use std::time::Duration;

    #[derive(Clone, Debug, Serialize, Deserialize)]
    struct FilterableEvent {
        id: u32,
        category: String,
        is_important: bool,
    }

    impl Event for FilterableEvent {
        fn event_type() -> &'static str {
            "filterable_event"
        }
        
        fn priority() -> EventPriority {
            EventPriority::Normal
        }
        
        fn as_any(&self) -> &dyn Any {
            self
        }
    }
    
    // Implement StaticEvent trait for FilterableEvent to work with StaticFastPath
    impl StaticEvent for FilterableEvent {}
    
    // Helper to register with the static registry
    fn register_event() {
        use crate::events::registry::GlobalTypeRegistry;
        GlobalTypeRegistry::register_static_event_type::<FilterableEvent>();
        GlobalTypeRegistry::register_with_capacity::<FilterableEvent>(1000);
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    // Test the subscribe_filtered method
    #[tokio::test]
    async fn test_subscribe_filtered() {
        // Register the event for static path
        register_event();
        
        // Test with both implementations
        for implementation in [ImplementationType::ZeroCopy, ImplementationType::StaticFastPath] {
            let system = EventSystemBuilder::new()
                .implementation(implementation)
                .channel_capacity(1000)
                .build();
                
            system.start().await.unwrap();
            
            // Create filtered subscriber for important events in category "A"
            let mut filtered_subscriber = system.subscribe_filtered::<FilterableEvent, _>(|event| {
                event.is_important && event.category == "A"
            }).await.unwrap();
            
            // Create publisher
            let publisher = system.create_publisher::<FilterableEvent>();
            
            // Publish a mix of events
            let events = vec![
                FilterableEvent { id: 1, category: "A".to_string(), is_important: true },   // Match
                FilterableEvent { id: 2, category: "A".to_string(), is_important: false },  // No match
                FilterableEvent { id: 3, category: "B".to_string(), is_important: true },   // No match
                FilterableEvent { id: 4, category: "A".to_string(), is_important: true },   // Match
            ];
            
            for event in events {
                publisher.publish(event).await.unwrap();
            }
            
            // Should receive only the 2 matching events
            let mut received_ids = Vec::new();
            
            for _ in 0..2 {
                match filtered_subscriber.receive_timeout(Duration::from_millis(100)).await {
                    Ok(event) => {
                        assert!(event.is_important, "Should only receive important events");
                        assert_eq!(event.category, "A", "Should only receive events from category A");
                        received_ids.push(event.id);
                    },
                    Err(_) => break,
                }
            }
            
            // Verify we got the matching events
            assert_eq!(received_ids.len(), 2);
            assert!(received_ids.contains(&1));
            assert!(received_ids.contains(&4));
            
            // No more events should be available
            match filtered_subscriber.receive_timeout(Duration::from_millis(50)).await {
                Ok(_) => panic!("Should not receive any more events"),
                Err(_) => (), // Expected timeout
            }
            
            system.shutdown().await.unwrap();
        }
    }
    
    // Test the subscribe_with_filter method
    #[tokio::test]
    async fn test_subscribe_with_filter() {
        // Register the event for static path
        register_event();
        
        // Test with both implementations
        for implementation in [ImplementationType::ZeroCopy, ImplementationType::StaticFastPath] {
            let system = EventSystemBuilder::new()
                .implementation(implementation)
                .channel_capacity(1000)
                .build();
                
            system.start().await.unwrap();
            
            // Create a filter using the utility functions
            use crate::events::api::filters;
            let category_filter = filters::field_equals(|e: &FilterableEvent| &e.category, "B".to_string());
            let importance_filter = filters::field_equals(|e: &FilterableEvent| &e.is_important, true);
            
            // Combine filters with AND
            let combined_filter = filters::and(category_filter, importance_filter);
            
            // Subscribe with the combined filter
            let mut filtered_subscriber = system.subscribe_with_filter::<FilterableEvent>(combined_filter).await.unwrap();
            
            // Create publisher
            let publisher = system.create_publisher::<FilterableEvent>();
            
            // Publish a mix of events
            let events = vec![
                FilterableEvent { id: 1, category: "A".to_string(), is_important: true },   // No match
                FilterableEvent { id: 2, category: "B".to_string(), is_important: false },  // No match
                FilterableEvent { id: 3, category: "B".to_string(), is_important: true },   // Match
                FilterableEvent { id: 4, category: "B".to_string(), is_important: true },   // Match
            ];
            
            for event in events {
                publisher.publish(event).await.unwrap();
            }
            
            // Should receive only the 2 matching events from category B that are important
            let mut received_ids = Vec::new();
            
            for _ in 0..2 {
                match filtered_subscriber.receive_timeout(Duration::from_millis(100)).await {
                    Ok(event) => {
                        assert!(event.is_important, "Should only receive important events");
                        assert_eq!(event.category, "B", "Should only receive events from category B");
                        received_ids.push(event.id);
                    },
                    Err(_) => break,
                }
            }
            
            // Verify we got the matching events
            assert_eq!(received_ids.len(), 2);
            assert!(received_ids.contains(&3));
            assert!(received_ids.contains(&4));
            
            system.shutdown().await.unwrap();
        }
    }
} 