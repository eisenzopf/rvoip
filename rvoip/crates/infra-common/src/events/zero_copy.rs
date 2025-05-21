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

use crate::events::types::{Event, EventResult, EventError};
use crate::events::bus::{EventBus, EventBusConfig, Publisher};
use crate::events::registry::TypedBroadcastReceiver;
use crate::events::api::{EventSystem, EventPublisher, EventSubscriber};

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
    receiver: TypedBroadcastReceiver<E>,
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