//! Core trait definitions for the unified event system API.
//!
//! This module defines the core interfaces that all event system implementations must satisfy.
//! These traits provide a common abstraction layer over different event system implementations
//! while allowing specialized optimizations for each implementation.

use std::sync::Arc;
use std::time::Duration;
use crate::events::types::{Event, EventResult, EventError, EventFilter};
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

    /// Subscribes to events of type `E` with a filter.
    ///
    /// This method creates a subscriber that only receives events that pass the filter.
    ///
    /// # Type Parameters
    ///
    /// * `E` - The event type to subscribe to
    ///
    /// # Arguments
    ///
    /// * `filter` - A function that takes a reference to an event and returns true if the event should be received
    ///
    /// # Returns
    ///
    /// A boxed subscriber that can receive filtered events of type `E`, or an error if
    /// subscription fails
    async fn subscribe_filtered<E, F>(&self, filter: F) -> EventResult<Box<dyn EventSubscriber<E>>>
    where
        E: Event + 'static,
        F: Fn(&E) -> bool + Send + Sync + 'static;

    /// Subscribes to events of type `E` with a predefined filter.
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
    /// A boxed subscriber that can receive filtered events of type `E`, or an error if
    /// subscription fails
    async fn subscribe_with_filter<E>(&self, filter: EventFilter<E>) -> EventResult<Box<dyn EventSubscriber<E>>>
    where
        E: Event + 'static;
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

/// Extension trait for creating filtered subscribers.
///
/// This trait is separate from EventSubscriber to maintain object safety.
pub trait FilterableSubscriber<E: Event>: Send {
    /// Creates a new subscriber that filters events according to the provided function.
    ///
    /// # Arguments
    ///
    /// * `filter_fn` - A function that takes a reference to an event and returns a boolean indicating
    ///                whether to accept it
    ///
    /// # Returns
    ///
    /// A new subscriber that only receives events that pass the filter
    fn with_filter<F>(&self, filter_fn: F) -> Box<dyn EventSubscriber<E>>
    where
        F: Fn(&E) -> bool + Send + Sync + 'static;
}

/// Utility functions for creating filters
pub mod filters {
    use super::*;
    use std::sync::Arc;
    
    /// Creates a filter that accepts events with a specific field value
    pub fn field_equals<E: Event, T, F>(field_extractor: F, value: T) -> EventFilter<E>
    where
        T: PartialEq + 'static + Send + Sync,
        F: Fn(&E) -> &T + Send + Sync + 'static,
    {
        Arc::new(move |event: &E| *field_extractor(event) == value)
    }
    
    /// Creates a filter that accepts events where a field satisfies a predicate
    pub fn field_matches<E: Event, T, F, P>(field_extractor: F, predicate: P) -> EventFilter<E>
    where
        F: Fn(&E) -> &T + Send + Sync + 'static,
        P: Fn(&T) -> bool + Send + Sync + 'static,
    {
        Arc::new(move |event: &E| predicate(field_extractor(event)))
    }
    
    /// Creates a filter that combines two filters with logical AND
    pub fn and<E: Event>(filter1: EventFilter<E>, filter2: EventFilter<E>) -> EventFilter<E> {
        Arc::new(move |event: &E| filter1(event) && filter2(event))
    }
    
    /// Creates a filter that combines two filters with logical OR
    pub fn or<E: Event>(filter1: EventFilter<E>, filter2: EventFilter<E>) -> EventFilter<E> {
        Arc::new(move |event: &E| filter1(event) || filter2(event))
    }
    
    /// Creates a filter that negates another filter
    pub fn not<E: Event>(filter: EventFilter<E>) -> EventFilter<E> {
        Arc::new(move |event: &E| !filter(event))
    }
}

/// Feature flag to enable static event system implementation.
pub const FEATURE_STATIC_EVENT_SYSTEM: &str = "static_event_system";

/// Feature flag to enable zero-copy event system implementation.
pub const FEATURE_ZERO_COPY_EVENT_SYSTEM: &str = "zero_copy_event_system";

#[cfg(test)]
mod tests {
    use super::*;
    use serde::{Serialize, Deserialize};
    use std::any::Any;
    use crate::events::types::EventPriority;

    #[derive(Clone, Debug, Serialize, Deserialize)]
    struct TestFilterEvent {
        id: u32,
        name: String,
        score: f64,
        tags: Vec<String>,
    }

    impl Event for TestFilterEvent {
        fn event_type() -> &'static str {
            "test_filter_event"
        }
        
        fn priority() -> EventPriority {
            EventPriority::Normal
        }
        
        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    #[test]
    fn test_field_equals_filter() {
        // Create a filter that matches events with id = 5
        let id_filter = filters::field_equals(|e: &TestFilterEvent| &e.id, 5);
        
        // Create some test events
        let event1 = TestFilterEvent {
            id: 5,
            name: "Match".to_string(),
            score: 10.0,
            tags: vec!["important".to_string()],
        };
        
        let event2 = TestFilterEvent {
            id: 10,
            name: "No Match".to_string(),
            score: 5.0,
            tags: vec!["important".to_string()],
        };
        
        // Test the filter
        assert!(id_filter(&event1), "Filter should match event with id=5");
        assert!(!id_filter(&event2), "Filter should not match event with id=10");
        
        // Create a filter for string field
        let name_filter = filters::field_equals(|e: &TestFilterEvent| &e.name, "Match".to_string());
        
        // Test string filter
        assert!(name_filter(&event1), "Filter should match event with name='Match'");
        assert!(!name_filter(&event2), "Filter should not match event with name='No Match'");
    }
    
    #[test]
    fn test_field_matches_filter() {
        // Create a filter that matches events with score > 7.0
        let score_filter = filters::field_matches(
            |e: &TestFilterEvent| &e.score,
            |score| *score > 7.0
        );
        
        // Create some test events
        let event1 = TestFilterEvent {
            id: 1,
            name: "High Score".to_string(),
            score: 9.5,
            tags: vec![],
        };
        
        let event2 = TestFilterEvent {
            id: 2,
            name: "Low Score".to_string(),
            score: 3.2,
            tags: vec![],
        };
        
        // Test the filter
        assert!(score_filter(&event1), "Filter should match event with score > 7.0");
        assert!(!score_filter(&event2), "Filter should not match event with score <= 7.0");
        
        // Create a filter that checks if tags contain "urgent"
        let tag_filter = filters::field_matches(
            |e: &TestFilterEvent| &e.tags,
            |tags| tags.contains(&"urgent".to_string())
        );
        
        // Create events with different tags
        let event3 = TestFilterEvent {
            id: 3,
            name: "Urgent".to_string(),
            score: 5.0,
            tags: vec!["important".to_string(), "urgent".to_string()],
        };
        
        let event4 = TestFilterEvent {
            id: 4,
            name: "Not Urgent".to_string(),
            score: 5.0,
            tags: vec!["normal".to_string()],
        };
        
        // Test the tag filter
        assert!(tag_filter(&event3), "Filter should match event with 'urgent' tag");
        assert!(!tag_filter(&event4), "Filter should not match event without 'urgent' tag");
    }
    
    #[test]
    fn test_logical_filter_operations() {
        // Create two simple filters
        let id_filter = filters::field_equals(|e: &TestFilterEvent| &e.id, 5);
        let score_filter = filters::field_matches(
            |e: &TestFilterEvent| &e.score,
            |score| *score > 7.0
        );
        
        // Create composite filters
        let and_filter = filters::and(id_filter.clone(), score_filter.clone());
        let or_filter = filters::or(id_filter.clone(), score_filter.clone());
        let not_id_filter = filters::not(id_filter.clone());
        
        // Create test events
        let event1 = TestFilterEvent {  // Matches both filters
            id: 5,
            name: "Both".to_string(),
            score: 9.0,
            tags: vec![],
        };
        
        let event2 = TestFilterEvent {  // Matches id_filter only
            id: 5,
            name: "Id Only".to_string(),
            score: 5.0,
            tags: vec![],
        };
        
        let event3 = TestFilterEvent {  // Matches score_filter only
            id: 10,
            name: "Score Only".to_string(),
            score: 8.0,
            tags: vec![],
        };
        
        let event4 = TestFilterEvent {  // Matches neither filter
            id: 1,
            name: "Neither".to_string(),
            score: 3.0,
            tags: vec![],
        };
        
        // Test AND filter
        assert!(and_filter(&event1), "AND filter should match when both conditions are true");
        assert!(!and_filter(&event2), "AND filter should not match when only id condition is true");
        assert!(!and_filter(&event3), "AND filter should not match when only score condition is true");
        assert!(!and_filter(&event4), "AND filter should not match when neither condition is true");
        
        // Test OR filter
        assert!(or_filter(&event1), "OR filter should match when both conditions are true");
        assert!(or_filter(&event2), "OR filter should match when only id condition is true");
        assert!(or_filter(&event3), "OR filter should match when only score condition is true");
        assert!(!or_filter(&event4), "OR filter should not match when neither condition is true");
        
        // Test NOT filter
        assert!(!not_id_filter(&event1), "NOT filter should not match when original filter matches");
        assert!(!not_id_filter(&event2), "NOT filter should not match when original filter matches");
        assert!(not_id_filter(&event3), "NOT filter should match when original filter doesn't match");
        assert!(not_id_filter(&event4), "NOT filter should match when original filter doesn't match");
    }
} 