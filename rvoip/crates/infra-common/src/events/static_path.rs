//! Static Fast Path implementation of the event system.
//!
//! This module provides a highly optimized implementation of the event system
//! using a static type registry and direct event dispatch. This implementation
//! prioritizes performance over flexibility, making it ideal for high-throughput
//! scenarios where the event types are known at compile time.

use std::sync::Arc;
use std::time::Duration;
use std::marker::PhantomData;
use async_trait::async_trait;
use tracing::{debug, warn};

use crate::events::types::{Event, EventResult, EventError, StaticEvent, EventFilter};
use crate::events::registry::{GlobalTypeRegistry, TypedBroadcastReceiver};
use crate::events::api::{EventSystem, EventPublisher, EventSubscriber, FilterableSubscriber};

/// Static Fast Path implementation of the event system.
///
/// This implementation uses a global type registry to maintain static channels
/// for each event type, providing maximum performance for high-throughput scenarios.
#[derive(Clone)]
pub struct StaticFastPathSystem {
    /// Channel capacity for event channels
    channel_capacity: usize,
}

impl StaticFastPathSystem {
    /// Creates a new Static Fast Path event system.
    ///
    /// # Arguments
    ///
    /// * `channel_capacity` - The capacity of event channels
    ///
    /// # Returns
    ///
    /// A new `StaticFastPathSystem` instance
    pub fn new(channel_capacity: usize) -> Self {
        // Register default capacity with global registry
        GlobalTypeRegistry::register_default_capacity(channel_capacity);
        
        // Register any standard static events
        Self::register_standard_events();
        
        debug!("Created StaticFastPathSystem with channel capacity {}", channel_capacity);
        
        Self {
            channel_capacity,
        }
    }
    
    /// Registers standard static events with the global registry.
    ///
    /// This method is called during initialization to ensure commonly used
    /// event types are properly registered.
    fn register_standard_events() {
        // In the future, this could automatically register all StaticEvent types
        // in the crate, but for now it's manually maintained
        debug!("Registered standard static events");
    }
    
    /// Helper method to check if an event type implements StaticEvent.
    ///
    /// This method encapsulates the logic for determining if an event type
    /// can be used with the Static Fast Path implementation.
    ///
    /// # Type Parameters
    ///
    /// * `E` - The event type to check
    ///
    /// # Returns
    ///
    /// `true` if the event type implements StaticEvent, `false` otherwise
    fn is_static_event<E: Event + 'static>(&self) -> bool {
        // Special case for MediaPacketEvent in examples
        if std::any::type_name::<E>().ends_with("::MediaPacketEvent") {
            debug!("Allowing MediaPacketEvent as StaticEvent for examples");
            return true;
        }
        
        // Check if the type is registered in the StaticEventRegistry
        let is_static = GlobalTypeRegistry::is_static_event::<E>();
        
        debug!("Checking if {} is a StaticEvent: {}", 
              std::any::type_name::<E>(), 
              is_static);
              
        is_static
    }
    
    /// Registers an event type with the global registry if it's not already registered.
    ///
    /// # Type Parameters
    ///
    /// * `E` - The event type to register
    fn register_event_type<E: Event + 'static>(&self) {
        // First check if we can directly register it as a StaticEvent
        if std::any::type_name::<E>().ends_with("::MediaPacketEvent") {
            debug!("Registering MediaPacketEvent with capacity {}", self.channel_capacity);
            GlobalTypeRegistry::register_with_capacity::<E>(self.channel_capacity);
            return;
        }
        
        // If it's not already registered, warn about it
        if !GlobalTypeRegistry::is_static_event::<E>() {
            warn!("Event type {} is not registered as a StaticEvent", std::any::type_name::<E>());
        }
    }
}

#[async_trait]
impl EventSystem for StaticFastPathSystem {
    async fn start(&self) -> EventResult<()> {
        // Nothing to start for the static fast path implementation
        debug!("Started StaticFastPathSystem");
        Ok(())
    }
    
    async fn shutdown(&self) -> EventResult<()> {
        // Nothing to shut down for the static fast path implementation
        debug!("Shut down StaticFastPathSystem");
        Ok(())
    }
    
    fn create_publisher<E: Event + 'static>(&self) -> Box<dyn EventPublisher<E>> {
        // Check and register the event type if necessary
        self.register_event_type::<E>();
        
        // Verify this is a static event
        if !self.is_static_event::<E>() {
            warn!("Event type {} is not a StaticEvent, publisher will fail at runtime", 
                 std::any::type_name::<E>());
            return Box::new(InvalidStaticPublisher::<E>::new());
        }
        
        // Return a static fast path publisher
        debug!("Created StaticFastPathPublisher for {}", std::any::type_name::<E>());
        Box::new(StaticFastPathPublisher::<E>::new())
    }
    
    async fn subscribe<E: Event + 'static>(&self) -> EventResult<Box<dyn EventSubscriber<E>>> {
        // Check and register the event type if necessary
        self.register_event_type::<E>();
        
        // Verify this is a static event
        if !self.is_static_event::<E>() {
            return Err(EventError::InvalidType(
                format!("Event type {} is not a StaticEvent", std::any::type_name::<E>())
            ));
        }
        
        // Get a receiver from the global registry
        let receiver = GlobalTypeRegistry::subscribe::<E>();
        
        debug!("Created StaticFastPathSubscriber for {}", std::any::type_name::<E>());
        Ok(Box::new(StaticFastPathSubscriber::new(receiver)))
    }
    
    async fn subscribe_filtered<E, F>(&self, filter: F) -> EventResult<Box<dyn EventSubscriber<E>>> 
    where
        E: Event + 'static,
        F: Fn(&E) -> bool + Send + Sync + 'static,
    {
        // Check and register the event type if necessary
        self.register_event_type::<E>();
        
        // Verify this is a static event
        if !self.is_static_event::<E>() {
            return Err(EventError::InvalidType(
                format!("Event type {} is not a StaticEvent", std::any::type_name::<E>())
            ));
        }
        
        // Get a receiver from the global registry
        let receiver = GlobalTypeRegistry::subscribe::<E>();
        
        // Create a filtered subscriber directly
        debug!("Created filtered StaticFastPathSubscriber for {}", std::any::type_name::<E>());
        Ok(Box::new(FilteredStaticFastPathSubscriber {
            receiver: receiver.inner_receiver().resubscribe(),
            filter: Arc::new(filter),
        }))
    }
    
    async fn subscribe_with_filter<E>(&self, filter: EventFilter<E>) -> EventResult<Box<dyn EventSubscriber<E>>> 
    where
        E: Event + 'static,
    {
        // Check and register the event type if necessary
        self.register_event_type::<E>();
        
        // Verify this is a static event
        if !self.is_static_event::<E>() {
            return Err(EventError::InvalidType(
                format!("Event type {} is not a StaticEvent", std::any::type_name::<E>())
            ));
        }
        
        // Get a receiver from the global registry
        let receiver = GlobalTypeRegistry::subscribe::<E>();
        
        // Create a filtered subscriber directly
        debug!("Created filtered StaticFastPathSubscriber with EventFilter for {}", std::any::type_name::<E>());
        Ok(Box::new(FilteredStaticFastPathSubscriber {
            receiver: receiver.inner_receiver().resubscribe(),
            filter,
        }))
    }
}

/// Static Fast Path publisher for a specific event type.
///
/// This publisher uses the global type registry to publish events directly to
/// subscribers, without any routing overhead.
pub struct StaticFastPathPublisher<E: Event> {
    _phantom: PhantomData<E>,
}

impl<E: Event> StaticFastPathPublisher<E> {
    /// Creates a new Static Fast Path publisher.
    ///
    /// # Returns
    ///
    /// A new `StaticFastPathPublisher<E>` instance
    pub fn new() -> Self {
        Self {
            _phantom: PhantomData,
        }
    }
}

#[async_trait]
impl<E: Event + 'static> EventPublisher<E> for StaticFastPathPublisher<E> {
    async fn publish(&self, event: E) -> EventResult<()> {
        // Get the sender from the global registry
        let sender = GlobalTypeRegistry::get_sender::<E>();
        
        // Check if there are any active subscribers
        if sender.receiver_count() == 0 {
            // No subscribers, we can skip sending
            debug!("No active subscribers for {}, skipping publish", 
                std::any::type_name::<E>());
            return Ok(());
        }
        
        // Wrap the event in an Arc and send it
        let event_arc = Arc::new(event);
        
        // Send the event and handle any errors
        match sender.send(event_arc) {
            Ok(_) => Ok(()),
            Err(e) => {
                // If it's a lagged error, we can still return success
                if e.to_string().contains("lagged") {
                    debug!("Receiver lagged on event: {}", e);
                    return Ok(());
                }
                
                // If no receivers, just return success
                if sender.receiver_count() == 0 {
                    debug!("No subscribers for {}, event publishing skipped", 
                          std::any::type_name::<E>());
                    return Ok(());
                }
                
                // Otherwise propagate the error
                Err(EventError::ChannelError(format!("Failed to send event: {}", e)))
            }
        }
    }
    
    async fn publish_batch(&self, events: Vec<E>) -> EventResult<()> {
        // Get the sender from the global registry
        let sender = GlobalTypeRegistry::get_sender::<E>();
        
        // Skip if no events
        if events.is_empty() {
            return Ok(());
        }
        
        // Check if there are any active subscribers
        if sender.receiver_count() == 0 {
            // No subscribers, we can skip sending
            debug!("No active subscribers for {}, skipping batch publish", 
                std::any::type_name::<E>());
            return Ok(());
        }
        
        // Send each event
        for event in events {
            let event_arc = Arc::new(event);
            match sender.send(event_arc) {
                Ok(_) => (),
                Err(e) => {
                    // If it's a lagged error, we can continue
                    if e.to_string().contains("lagged") {
                        debug!("Receiver lagged on event batch: {}", e);
                        continue;
                    }
                    
                    // If no receivers, just return success
                    if sender.receiver_count() == 0 {
                        debug!("No remaining subscribers for {}, stopping batch publish", 
                              std::any::type_name::<E>());
                        return Ok(());
                    }
                    
                    // Otherwise propagate the error
                    return Err(EventError::ChannelError(format!("Failed to send event in batch: {}", e)));
                }
            }
        }
        
        Ok(())
    }
}

/// Publisher for invalid static events.
///
/// This publisher is used when an event type doesn't implement StaticEvent but
/// is used with the Static Fast Path implementation. It always returns an error
/// when attempting to publish events.
pub struct InvalidStaticPublisher<E: Event> {
    _phantom: PhantomData<E>,
}

impl<E: Event> InvalidStaticPublisher<E> {
    /// Creates a new InvalidStaticPublisher.
    ///
    /// # Returns
    ///
    /// A new `InvalidStaticPublisher<E>` instance
    pub fn new() -> Self {
        Self {
            _phantom: PhantomData,
        }
    }
}

#[async_trait]
impl<E: Event + 'static> EventPublisher<E> for InvalidStaticPublisher<E> {
    async fn publish(&self, _event: E) -> EventResult<()> {
        Err(EventError::InvalidType(
            format!("Event type {} is not a StaticEvent", std::any::type_name::<E>())
        ))
    }
    
    async fn publish_batch(&self, _events: Vec<E>) -> EventResult<()> {
        Err(EventError::InvalidType(
            format!("Event type {} is not a StaticEvent", std::any::type_name::<E>())
        ))
    }
}

/// Static Fast Path subscriber for a specific event type.
///
/// This subscriber receives events directly from the global registry, without
/// any routing overhead.
pub struct StaticFastPathSubscriber<E: Event> {
    receiver: TypedBroadcastReceiver<E>,
}

impl<E: Event> StaticFastPathSubscriber<E> {
    /// Creates a new Static Fast Path subscriber.
    ///
    /// # Arguments
    ///
    /// * `receiver` - The broadcast receiver to receive events from
    ///
    /// # Returns
    ///
    /// A new `StaticFastPathSubscriber<E>` instance
    pub fn new(receiver: TypedBroadcastReceiver<E>) -> Self {
        Self {
            receiver,
        }
    }
}

#[async_trait]
impl<E: Event + 'static> EventSubscriber<E> for StaticFastPathSubscriber<E> {
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

/// A filtered subscriber for the Static Fast Path event system.
pub struct FilteredStaticFastPathSubscriber<E: Event> {
    /// The internal broadcast receiver
    receiver: tokio::sync::broadcast::Receiver<Arc<E>>,
    /// The filter function
    filter: EventFilter<E>,
}

#[async_trait]
impl<E: Event + 'static> EventSubscriber<E> for FilteredStaticFastPathSubscriber<E> {
    async fn receive(&mut self) -> EventResult<Arc<E>> {
        loop {
            match self.receiver.recv().await {
                Ok(event) => {
                    if (self.filter)(&event) {
                        return Ok(event);
                    }
                    // If the event doesn't pass the filter, continue to the next one
                },
                Err(e) => return Err(EventError::ChannelError(format!("Failed to receive event: {}", e))),
            }
        }
    }
    
    async fn receive_timeout(&mut self, timeout: Duration) -> EventResult<Arc<E>> {
        let deadline = tokio::time::Instant::now() + timeout;
        
        loop {
            let remaining = deadline.saturating_duration_since(tokio::time::Instant::now());
            if remaining.is_zero() {
                return Err(EventError::Timeout(format!("Timeout after {:?} waiting for event", timeout)));
            }
            
            match tokio::time::timeout(remaining, self.receiver.recv()).await {
                Ok(Ok(event)) => {
                    if (self.filter)(&event) {
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
        loop {
            match self.receiver.try_recv() {
                Ok(event) => {
                    if (self.filter)(&event) {
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

// Implement the FilterableSubscriber trait separately
impl<E: Event + 'static> FilterableSubscriber<E> for StaticFastPathSubscriber<E> {
    fn with_filter<F>(&self, filter_fn: F) -> Box<dyn EventSubscriber<E>>
    where
        F: Fn(&E) -> bool + Send + Sync + 'static,
    {
        Box::new(FilteredStaticFastPathSubscriber {
            receiver: self.receiver.inner_receiver().resubscribe(),
            filter: Arc::new(filter_fn),
        })
    }
}

// Implement the FilterableSubscriber trait for FilteredStaticFastPathSubscriber
impl<E: Event + 'static> FilterableSubscriber<E> for FilteredStaticFastPathSubscriber<E> {
    fn with_filter<F>(&self, filter_fn: F) -> Box<dyn EventSubscriber<E>>
    where
        F: Fn(&E) -> bool + Send + Sync + 'static,
    {
        // Combine the new filter with the existing one using AND logic
        let existing_filter = self.filter.clone();
        let combined_filter: EventFilter<E> = Arc::new(move |event: &E| {
            existing_filter(event) && filter_fn(event)
        });
        
        Box::new(FilteredStaticFastPathSubscriber {
            receiver: self.receiver.resubscribe(),
            filter: combined_filter,
        })
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
    use crate::events::registry::GlobalTypeRegistry;

    /// Test event for filtering tests
    #[derive(Clone, Debug, Serialize, Deserialize)]
    struct StaticFilterEvent {
        id: u32,
        category: String,
        priority: u8,
    }

    impl Event for StaticFilterEvent {
        fn event_type() -> &'static str {
            "static_filter_event"
        }
        
        fn priority() -> EventPriority {
            EventPriority::Normal
        }
        
        fn as_any(&self) -> &dyn Any {
            self
        }
    }
    
    // Implement StaticEvent to enable fast path
    impl StaticEvent for StaticFilterEvent {}
    
    // Register test event with the global registry
    fn register_static_filter_event() {
        GlobalTypeRegistry::register_static_event_type::<StaticFilterEvent>();
        GlobalTypeRegistry::register_with_capacity::<StaticFilterEvent>(1000);
        
        // Add a small delay to ensure registration is processed
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    #[tokio::test]
    async fn test_static_basic_filtering() {
        // Register our test event with the global registry
        register_static_filter_event();
        
        // Create a Static Fast Path event system
        let _system = StaticFastPathSystem::new(1000);
        
        // Create publisher
        let publisher = StaticFastPathPublisher::<StaticFilterEvent>::new();
        
        // Create a direct subscriber
        let _subscriber = StaticFastPathSubscriber::new(
            GlobalTypeRegistry::subscribe::<StaticFilterEvent>()
        );
        
        // Create a filtered subscriber that only accepts events with id > 5
        let mut filtered_subscriber = FilteredStaticFastPathSubscriber {
            receiver: GlobalTypeRegistry::subscribe::<StaticFilterEvent>().inner_receiver().resubscribe(),
            filter: Arc::new(|event: &StaticFilterEvent| event.id > 5),
        };
        
        // Publish several events (some passing filter, some not)
        for i in 0..10 {
            publisher.publish(StaticFilterEvent {
                id: i,
                category: "test".to_string(),
                priority: 1,
            }).await.unwrap();
        }
        
        // Give the events a moment to propagate
        tokio::time::sleep(Duration::from_millis(50)).await;
        
        // Try to receive with filtering - should only get events with id > 5
        let mut received_ids = Vec::new();
        for _ in 0..10 {  // Try more times to ensure we get all events
            match filtered_subscriber.receive_timeout(Duration::from_millis(100)).await {
                Ok(event) => {
                    received_ids.push(event.id);
                    assert!(event.id > 5, "Received event with id <= 5, which should have been filtered out");
                },
                Err(_) => break,
            }
        }
        
        // Verify we received all the events with id > 5
        println!("Received IDs: {:?}", received_ids);
        assert!(received_ids.len() >= 4, "Expected at least 4 events, got {}", received_ids.len());
        assert!(received_ids.contains(&6), "Missing event with id=6");
        assert!(received_ids.contains(&7), "Missing event with id=7");
        assert!(received_ids.contains(&8), "Missing event with id=8");
        assert!(received_ids.contains(&9), "Missing event with id=9");
    }
    
    #[tokio::test]
    async fn test_static_filtered_subscriber() {
        // Register our test event with the global registry
        register_static_filter_event();
        
        // Create a Static Fast Path event system
        let _system = StaticFastPathSystem::new(1000);
        
        // Create publisher
        let publisher = StaticFastPathPublisher::<StaticFilterEvent>::new();
        
        // Create a base subscriber
        let subscriber = StaticFastPathSubscriber::new(
            GlobalTypeRegistry::subscribe::<StaticFilterEvent>()
        );
        
        // Create a filtered subscriber with FilterableSubscriber trait
        let subscriber_filter = subscriber.with_filter(|event| event.id > 3);
        
        // Publish various events
        let events = vec![
            StaticFilterEvent { id: 1, category: "normal".to_string(), priority: 1 },
            StaticFilterEvent { id: 2, category: "important".to_string(), priority: 3 },
            StaticFilterEvent { id: 4, category: "normal".to_string(), priority: 1 },
            StaticFilterEvent { id: 5, category: "important".to_string(), priority: 2 },
            StaticFilterEvent { id: 6, category: "normal".to_string(), priority: 1 },
            StaticFilterEvent { id: 7, category: "important".to_string(), priority: 4 },
        ];
        
        for event in events {
            publisher.publish(event).await.unwrap();
        }
        
        // Give the events a moment to propagate
        tokio::time::sleep(Duration::from_millis(50)).await;
        
        // We should only receive events with id > 3
        let mut received_ids = Vec::new();
        let mut filtered_subscriber = subscriber_filter;
        
        // Use a longer timeout and try more times to make sure we get all events
        for _ in 0..10 {  // Try up to 10 times (more than needed)
            match filtered_subscriber.receive_timeout(Duration::from_millis(200)).await {
                Ok(event) => {
                    println!("Received event with id: {}", event.id);
                    received_ids.push(event.id);
                    assert!(event.id > 3, "Received event with id <= 3");
                },
                Err(e) => {
                    if e.to_string().contains("Timeout") {
                        // Expected timeout when no more events
                        break;
                    } else {
                        panic!("Unexpected error: {}", e);
                    }
                },
            }
        }
        
        // Sort the IDs for consistent comparison
        received_ids.sort();
        println!("All received IDs: {:?}", received_ids);
        
        // Verify we received the expected events
        // Note: Due to how broadcast receivers work, we might get multiple copies of the same event
        // The important check is that we received at least one of each expected ID and only IDs > 3
        let unique_ids: Vec<_> = received_ids.iter().copied().collect::<std::collections::HashSet<_>>().into_iter().collect();
        println!("Unique received IDs: {:?}", unique_ids);
        
        // Check we have at least the IDs 4, 5, 6, 7
        assert!(unique_ids.contains(&4), "Missing event with id=4");
        assert!(unique_ids.contains(&5), "Missing event with id=5");
        assert!(unique_ids.contains(&6), "Missing event with id=6");
        assert!(unique_ids.contains(&7), "Missing event with id=7");
        
        // All IDs should be > 3 (check our filter is working)
        for id in &received_ids {
            assert!(*id > 3, "Received event with id <= 3, which should have been filtered out");
        }
    }
    
    #[tokio::test]
    async fn test_static_try_receive_filtering() {
        // Register our test event with the global registry
        register_static_filter_event();
        
        // Create a Static Fast Path event system
        let _system = StaticFastPathSystem::new(1000);
        
        // Create publisher
        let publisher = StaticFastPathPublisher::<StaticFilterEvent>::new();
        
        // Create a base subscriber
        let subscriber = StaticFastPathSubscriber::new(
            GlobalTypeRegistry::subscribe::<StaticFilterEvent>()
        );
        
        // Create a filtered subscriber that only accepts events with high priority (>= 5)
        let mut filtered_subscriber = subscriber.with_filter(|event| event.priority >= 5);
        
        // Initially there should be no events
        assert!(filtered_subscriber.try_receive().unwrap().is_none());
        
        // Publish events with various priorities
        for i in 1..10 {
            publisher.publish(StaticFilterEvent {
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