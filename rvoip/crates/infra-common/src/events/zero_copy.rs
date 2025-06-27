//! Zero Copy implementation of the event system.
//!
//! This module provides a feature-rich implementation of the event system
//! using a zero-copy event bus. This implementation prioritizes flexibility
//! and features over raw performance, making it ideal for scenarios that
//! need advanced routing, filtering, or other features.

use std::sync::Arc;
use std::time::Duration;
use async_trait::async_trait;
use tracing::debug;

use crate::events::types::{Event, EventResult, EventError, EventFilter};
use crate::events::bus::{EventBus, EventBusConfig, Publisher};
use crate::events::registry::TypedBroadcastReceiver;
use crate::events::api::{EventSystem, EventPublisher, EventSubscriber, FilterableSubscriber};

/// Zero Copy implementation of the event system.
///
/// This implementation uses a feature-rich event bus that supports advanced
/// routing, filtering, and other features.
#[derive(Clone)]
pub struct ZeroCopySystem {
    /// The underlying event bus
    event_bus: EventBus,
}

impl ZeroCopySystem {
    /// Creates a new Zero Copy event system with the specified configuration.
    ///
    /// # Arguments
    ///
    /// * `config` - The configuration for the event bus
    ///
    /// # Returns
    ///
    /// A new `ZeroCopySystem` instance
    pub fn new(config: EventBusConfig) -> Self {
        debug!("Created ZeroCopySystem with config: {:?}", config);
        
        Self {
            event_bus: EventBus::with_config(config),
        }
    }
    
    /// Creates a new Zero Copy event system with default configuration.
    ///
    /// # Returns
    ///
    /// A new `ZeroCopySystem` instance with default configuration
    pub fn new_default() -> Self {
        let config = EventBusConfig {
            broadcast_capacity: 10_000,
            max_concurrent_dispatches: 1_000,
            enable_priority: true,
            default_timeout: Duration::from_secs(1),
            enable_zero_copy: true,
            batch_size: 100,
            shard_count: 8,
        };
        
        Self::new(config)
    }
    
    /// Returns a reference to the underlying event bus.
    ///
    /// This method provides access to the underlying event bus for advanced
    /// operations that aren't available through the unified API.
    ///
    /// # Returns
    ///
    /// A reference to the underlying event bus
    pub fn event_bus(&self) -> &EventBus {
        &self.event_bus
    }
}

#[async_trait]
impl EventSystem for ZeroCopySystem {
    async fn start(&self) -> EventResult<()> {
        // Start the event bus (currently a no-op)
        debug!("Started ZeroCopySystem");
        Ok(())
    }
    
    async fn shutdown(&self) -> EventResult<()> {
        // Shut down the event bus (currently a no-op)
        debug!("Shut down ZeroCopySystem");
        Ok(())
    }
    
    fn create_publisher<E: Event + 'static>(&self) -> Box<dyn EventPublisher<E>> {
        debug!("Created ZeroCopyPublisher for {}", std::any::type_name::<E>());
        Box::new(ZeroCopyPublisher::new(self.event_bus.clone()))
    }
    
    async fn subscribe<E: Event + 'static>(&self) -> EventResult<Box<dyn EventSubscriber<E>>> {
        // Subscribe to the event using the event bus
        let receiver = self.event_bus.subscribe_broadcast::<E>().await?;
        
        debug!("Created ZeroCopySubscriber for {}", std::any::type_name::<E>());
        Ok(Box::new(ZeroCopySubscriber::new(receiver)))
    }
    
    async fn subscribe_filtered<E, F>(&self, filter: F) -> EventResult<Box<dyn EventSubscriber<E>>> 
    where
        E: Event + 'static,
        F: Fn(&E) -> bool + Send + Sync + 'static,
    {
        // Subscribe to the event using the event bus
        let receiver = self.event_bus.subscribe_broadcast::<E>().await?;
        
        debug!("Created filtered ZeroCopySubscriber for {}", std::any::type_name::<E>());
        
        // Create a filtered subscriber directly
        Ok(Box::new(FilteredZeroCopySubscriber::new(
            receiver,
            Arc::new(filter)
        )))
    }
    
    async fn subscribe_with_filter<E>(&self, filter: EventFilter<E>) -> EventResult<Box<dyn EventSubscriber<E>>> 
    where
        E: Event + 'static,
    {
        // Subscribe to the event using the event bus
        let receiver = self.event_bus.subscribe_broadcast::<E>().await?;
        
        debug!("Created filtered ZeroCopySubscriber with EventFilter for {}", std::any::type_name::<E>());
        
        // Create a filtered subscriber directly
        Ok(Box::new(FilteredZeroCopySubscriber::new(
            receiver,
            filter
        )))
    }
}

/// Zero Copy publisher for a specific event type.
///
/// This publisher uses the event bus to publish events, taking advantage of
/// its routing and filtering capabilities.
pub struct ZeroCopyPublisher<E: Event> {
    /// The underlying publisher
    publisher: Publisher<E>,
}

impl<E: Event> ZeroCopyPublisher<E> {
    /// Creates a new Zero Copy publisher.
    ///
    /// # Arguments
    ///
    /// * `event_bus` - The event bus to publish to
    ///
    /// # Returns
    ///
    /// A new `ZeroCopyPublisher<E>` instance
    pub fn new(event_bus: EventBus) -> Self {
        Self {
            publisher: Publisher::new(event_bus),
        }
    }
}

#[async_trait]
impl<E: Event + 'static> EventPublisher<E> for ZeroCopyPublisher<E> {
    async fn publish(&self, event: E) -> EventResult<()> {
        self.publisher.publish(event).await
    }
    
    async fn publish_batch(&self, events: Vec<E>) -> EventResult<()> {
        self.publisher.publish_batch(events).await
    }
}

/// Zero Copy subscriber for a specific event type.
///
/// This subscriber receives events from the event bus, taking advantage of
/// its routing and filtering capabilities.
pub struct ZeroCopySubscriber<E: Event> {
    /// The underlying receiver
    pub(crate) receiver: TypedBroadcastReceiver<E>,
}

impl<E: Event> ZeroCopySubscriber<E> {
    /// Creates a new Zero Copy subscriber.
    ///
    /// # Arguments
    ///
    /// * `receiver` - The broadcast receiver to receive events from
    ///
    /// # Returns
    ///
    /// A new `ZeroCopySubscriber<E>` instance
    pub fn new(receiver: TypedBroadcastReceiver<E>) -> Self {
        Self {
            receiver,
        }
    }
}

#[async_trait]
impl<E: Event + 'static> EventSubscriber<E> for ZeroCopySubscriber<E> {
    async fn receive(&mut self) -> EventResult<Arc<E>> {
        self.receiver.recv().await
            .map_err(|e| EventError::ChannelError(format!("Failed to receive event: {}", e)))
    }
    
    async fn receive_timeout(&mut self, timeout: Duration) -> EventResult<Arc<E>> {
        match tokio::time::timeout(timeout, self.receive()).await {
            Ok(result) => result,
            Err(_) => Err(EventError::Timeout(format!("Timeout after {:?} waiting for event", timeout))),
        }
    }
    
    fn try_receive(&mut self) -> EventResult<Option<Arc<E>>> {
        match self.receiver.try_recv() {
            Ok(event) => Ok(Some(event)),
            Err(tokio::sync::broadcast::error::TryRecvError::Empty) => Ok(None),
            Err(e) => Err(EventError::ChannelError(format!("Failed to try_receive event: {}", e))),
        }
    }
}

// Implement the extension trait for filtering
impl<E: Event + 'static> FilterableSubscriber<E> for ZeroCopySubscriber<E> {
    fn with_filter<F>(&self, filter_fn: F) -> Box<dyn EventSubscriber<E>>
    where
        F: Fn(&E) -> bool + Send + Sync + 'static,
    {
        // Create a new filtered subscriber with a cloned receiver
        Box::new(FilteredZeroCopySubscriber::new(
            TypedBroadcastReceiver::new(self.receiver.inner_receiver().resubscribe()),
            Arc::new(filter_fn)
        ))
    }
}

/// A filtered zero copy subscriber for a specific event type.
///
/// This subscriber wraps a ZeroCopySubscriber and applies a filter to
/// only return events that match the filter.
pub struct FilteredZeroCopySubscriber<E: Event> {
    /// The underlying receiver
    receiver: TypedBroadcastReceiver<E>,
    /// The filter function
    filter: EventFilter<E>,
}

impl<E: Event> FilteredZeroCopySubscriber<E> {
    /// Creates a new filtered zero copy subscriber.
    ///
    /// # Arguments
    ///
    /// * `receiver` - The broadcast receiver to receive events from
    /// * `filter` - The filter function to apply
    ///
    /// # Returns
    ///
    /// A new `FilteredZeroCopySubscriber<E>` instance
    pub fn new(receiver: TypedBroadcastReceiver<E>, filter: EventFilter<E>) -> Self {
        Self {
            receiver,
            filter,
        }
    }
    
    /// Helper method to check if an event passes the filter
    fn passes_filter(&self, event: &Arc<E>) -> bool {
        (self.filter)(event)
    }
}

#[async_trait]
impl<E: Event + 'static> EventSubscriber<E> for FilteredZeroCopySubscriber<E> {
    async fn receive(&mut self) -> EventResult<Arc<E>> {
        // Keep receiving events until one passes the filter
        loop {
            let event = self.receiver.recv().await
                .map_err(|e| EventError::ChannelError(format!("Failed to receive event: {}", e)))?;
            
            if self.passes_filter(&event) {
                return Ok(event);
            }
            
            // If the event doesn't pass the filter, continue to the next one
        }
    }
    
    async fn receive_timeout(&mut self, timeout: Duration) -> EventResult<Arc<E>> {
        // Use a deadline to respect the total timeout
        let deadline = tokio::time::Instant::now() + timeout;
        
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(EventError::Timeout(format!("Timeout after {:?} waiting for event", timeout)));
            }
            
            // Try to receive an event with the remaining time
            match tokio::time::timeout(remaining, self.receiver.recv()).await {
                Ok(Ok(event)) => {
                    if self.passes_filter(&event) {
                        return Ok(event);
                    }
                    // If the event doesn't pass the filter, continue to the next one
                },
                Ok(Err(e)) => return Err(EventError::ChannelError(format!("Failed to receive event: {}", e))),
                Err(_) => return Err(EventError::Timeout(format!("Timeout after {:?} waiting for event", timeout))),
            }
        }
    }
    
    fn try_receive(&mut self) -> EventResult<Option<Arc<E>>> {
        // Try to receive all available events until we find one that passes the filter
        loop {
            match self.receiver.try_recv() {
                Ok(event) => {
                    if self.passes_filter(&event) {
                        return Ok(Some(event));
                    }
                    // If the event doesn't pass the filter, try the next one
                },
                Err(tokio::sync::broadcast::error::TryRecvError::Empty) => return Ok(None),
                Err(e) => return Err(EventError::ChannelError(format!("Failed to try_receive event: {}", e))),
            }
        }
    }
}

// Implement the extension trait for FilteredZeroCopySubscriber
impl<E: Event + 'static> FilterableSubscriber<E> for FilteredZeroCopySubscriber<E> {
    fn with_filter<F>(&self, filter_fn: F) -> Box<dyn EventSubscriber<E>>
    where
        F: Fn(&E) -> bool + Send + Sync + 'static,
    {
        // Combine the new filter with the existing one using AND logic
        let existing_filter = self.filter.clone();
        let combined_filter: EventFilter<E> = Arc::new(move |event: &E| {
            existing_filter(event) && filter_fn(event)
        });
        
        // Create a new filtered subscriber with the combined filter
        Box::new(FilteredZeroCopySubscriber::new(
            TypedBroadcastReceiver::new(self.receiver.inner_receiver().resubscribe()),
            combined_filter
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::events::types::EventPriority;
    use serde::{Serialize, Deserialize};
    use std::any::Any;
    use std::sync::Arc;
    use std::time::Duration;

    /// Test event for filtering tests
    #[derive(Clone, Debug, Serialize, Deserialize)]
    struct FilterTestEvent {
        id: u32,
        category: String,
        priority: u8,
    }

    impl Event for FilterTestEvent {
        fn event_type() -> &'static str {
            "filter_test_event"
        }
        
        fn priority() -> EventPriority {
            EventPriority::Normal
        }
        
        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    #[tokio::test]
    async fn test_basic_filtering() {
        // Create a Zero Copy event system
        let system = ZeroCopySystem::new_default();
        let event_bus = system.event_bus().clone();
        
        // Create publisher
        let publisher = Publisher::<FilterTestEvent>::new(event_bus.clone());
        
        // Subscribe and get the receiver
        let receiver = event_bus.subscribe_broadcast::<FilterTestEvent>().await.unwrap();
        
        // Create a filtered subscriber directly
        let mut filtered_subscriber = FilteredZeroCopySubscriber::new(
            receiver,
            Arc::new(|event: &FilterTestEvent| event.id > 5)
        );
        
        // Publish several events (some passing filter, some not)
        for i in 0..10 {
            publisher.publish(FilterTestEvent {
                id: i,
                category: "test".to_string(),
                priority: 1,
            }).await.unwrap();
        }
        
        // Try to receive with filtering - should only get events with id > 5
        let mut received_ids = Vec::new();
        for _ in 0..4 {
            match filtered_subscriber.receive_timeout(Duration::from_millis(100)).await {
                Ok(event) => {
                    received_ids.push(event.id);
                    assert!(event.id > 5, "Received event with id <= 5, which should have been filtered out");
                },
                Err(_) => break,
            }
        }
        
        // Verify we received all the events with id > 5
        assert_eq!(received_ids.len(), 4);
        assert!(received_ids.contains(&6));
        assert!(received_ids.contains(&7));
        assert!(received_ids.contains(&8));
        assert!(received_ids.contains(&9));
    }
    
    #[tokio::test]
    async fn test_filtered_subscriber() {
        // Create a Zero Copy event system
        let system = ZeroCopySystem::new_default();
        let event_bus = system.event_bus().clone();
        
        // Create publisher
        let publisher = Publisher::<FilterTestEvent>::new(event_bus.clone());
        
        // Subscribe and get the receiver
        let receiver = event_bus.subscribe_broadcast::<FilterTestEvent>().await.unwrap();
        
        // Create the base subscriber
        let subscriber = ZeroCopySubscriber::new(receiver);
        
        // Create a filtered subscriber through the trait
        let subscriber_filter = subscriber.with_filter(|event| event.id > 3);
        
        // Adding another filter isn't possible through Box<dyn EventSubscriber>,
        // we would need to use a different approach for that
        
        // Publish various events
        let events = vec![
            FilterTestEvent { id: 1, category: "normal".to_string(), priority: 1 },
            FilterTestEvent { id: 2, category: "important".to_string(), priority: 3 },
            FilterTestEvent { id: 4, category: "normal".to_string(), priority: 1 },
            FilterTestEvent { id: 5, category: "important".to_string(), priority: 2 },
            FilterTestEvent { id: 6, category: "normal".to_string(), priority: 1 },
            FilterTestEvent { id: 7, category: "important".to_string(), priority: 4 },
        ];
        
        for event in events {
            publisher.publish(event).await.unwrap();
        }
        
        // We should only receive events with id > 3 
        let mut received_ids = Vec::new();
        let mut filtered_subscriber = subscriber_filter;
        
        for _ in 0..4 {
            match filtered_subscriber.receive_timeout(Duration::from_millis(100)).await {
                Ok(event) => {
                    received_ids.push(event.id);
                    assert!(event.id > 3, "Received event with id <= 3");
                },
                Err(_) => break,
            }
        }
        
        // Verify we received only the events that pass the filter
        assert_eq!(received_ids.len(), 4);
        assert!(received_ids.contains(&4));
        assert!(received_ids.contains(&5));
        assert!(received_ids.contains(&6));
        assert!(received_ids.contains(&7));
    }
    
    #[tokio::test]
    async fn test_try_receive_filtering() {
        // Create a Zero Copy event system
        let system = ZeroCopySystem::new_default();
        let event_bus = system.event_bus().clone();
        
        // Create publisher
        let publisher = Publisher::<FilterTestEvent>::new(event_bus.clone());
        
        // Subscribe and get the receiver
        let receiver = event_bus.subscribe_broadcast::<FilterTestEvent>().await.unwrap();
        
        // Create the base subscriber
        let subscriber = ZeroCopySubscriber::new(receiver);
        
        // Create a filtered subscriber that only accepts events with high priority (>= 5)
        let mut filtered_subscriber = subscriber.with_filter(|event| event.priority >= 5);
        
        // Initially there should be no events
        assert!(filtered_subscriber.try_receive().unwrap().is_none());
        
        // Publish events with various priorities
        for i in 1..10 {
            publisher.publish(FilterTestEvent {
                id: i,
                category: "test".to_string(),
                priority: i as u8,
            }).await.unwrap();
        }
        
        // Wait a moment for events to be processed
        tokio::time::sleep(Duration::from_millis(50)).await;
        
        // We should only receive events with priority >= 5
        let mut high_priority_events = Vec::new();
        while let Ok(Some(event)) = filtered_subscriber.try_receive() {
            assert!(event.priority >= 5, "Received event with priority < 5");
            high_priority_events.push(event.id);
        }
        
        // Verify we received only the high priority events
        assert_eq!(high_priority_events.len(), 5); // events with priority 5-9
        for i in 5..10 {
            assert!(high_priority_events.contains(&i));
        }
    }
} 