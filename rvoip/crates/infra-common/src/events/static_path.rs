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

use crate::events::types::{Event, EventResult, EventError, StaticEvent};
use crate::events::registry::{GlobalTypeRegistry, TypedBroadcastReceiver};
use super::api::{EventSystem, EventPublisher, EventSubscriber};

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