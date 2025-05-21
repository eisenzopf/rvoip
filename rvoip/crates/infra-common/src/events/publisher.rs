use crate::events::bus::EventBus;
use crate::events::types::{Event, EventResult, StaticEvent};
use crate::events::registry::{TypedBroadcastSender, TypedBroadcastReceiver, GlobalTypeRegistry};
use std::marker::PhantomData;
use std::time::Duration;
use std::sync::Arc;

/// Publisher for a specific event type
pub struct Publisher<E: Event> {
    event_bus: EventBus,
    _phantom: PhantomData<E>,
    default_timeout: Option<Duration>,
    typed_sender: TypedBroadcastSender<E>,
}

impl<E: Event> Publisher<E> {
    /// Create a new publisher that will emit events to the provided event bus
    pub fn new(event_bus: EventBus) -> Self {
        let typed_sender = event_bus.type_registry().get_or_create::<E>();
        Publisher {
            event_bus,
            _phantom: PhantomData,
            default_timeout: None,
            typed_sender,
        }
    }
    
    /// Create a new publisher with a custom timeout
    pub fn with_timeout(event_bus: EventBus, timeout: Duration) -> Self {
        let typed_sender = event_bus.type_registry().get_or_create::<E>();
        Publisher {
            event_bus,
            _phantom: PhantomData,
            default_timeout: Some(timeout),
            typed_sender,
        }
    }
    
    /// Publish an event using zero-copy when possible
    pub async fn publish(&self, event: E) -> EventResult<()> {
        let arc_event = Arc::new(event.clone());
        match self.typed_sender.send(arc_event) {
            Ok(_) => return Ok(()),
            Err(_) => {
                if let Some(timeout) = self.default_timeout {
                    self.event_bus.publish_with_timeout(event, timeout).await
                } else {
                    self.event_bus.publish(event).await
                }
            }
        }
    }
    
    /// Publish a batch of events for high throughput
    pub async fn publish_batch(&self, events: Vec<E>) -> EventResult<()> {
        self.event_bus.publish_batch(events).await
    }
    
    /// Create a channel for sending events
    pub fn create_channel(&self) -> tokio::sync::mpsc::Sender<E> {
        self.event_bus.create_channel()
    }
    
    /// Get a channel subscription for this event type
    pub async fn subscribe_broadcast(&self) -> EventResult<TypedBroadcastReceiver<E>> {
        self.event_bus.subscribe_broadcast().await
    }
    
    /// Get a direct broadcast sender for this event type
    pub fn get_broadcast_sender(&self) -> TypedBroadcastSender<E> {
        self.event_bus.type_registry().get_or_create::<E>()
    }
}

/// Fast publisher for static events with cached type information
pub struct FastPublisher<E: StaticEvent> {
    _phantom: PhantomData<E>,
    sender: TypedBroadcastSender<E>,
}

impl<E: StaticEvent> FastPublisher<E> {
    /// Create a new fast publisher
    pub fn new() -> Self {
        let sender = GlobalTypeRegistry::get_sender::<E>();
        Self {
            _phantom: PhantomData,
            sender,
        }
    }
    
    /// Create a new fast publisher with custom channel capacity
    pub fn with_capacity(capacity: usize) -> Self {
        let sender = GlobalTypeRegistry::register_with_capacity::<E>(capacity);
        
        Self {
            _phantom: PhantomData,
            sender,
        }
    }
    
    /// Publish an event using the global type registry
    pub async fn publish(&self, event: E) -> EventResult<()> {
        let arc_event = Arc::new(event);
        
        match self.sender.send(arc_event) {
            Ok(_) => Ok(()),
            Err(err) => Err(crate::events::types::EventError::ChannelError(
                format!("Fast broadcast failed: {}", err)
            )),
        }
    }
    
    /// Get a broadcast receiver for this event type
    pub fn subscribe(&self) -> TypedBroadcastReceiver<E> {
        TypedBroadcastReceiver::new(self.sender.subscribe())
    }
}

/// Factory for creating typed publishers
#[derive(Clone)]
pub struct PublisherFactory {
    event_bus: EventBus,
    default_timeout: Option<Duration>,
}

impl PublisherFactory {
    /// Create a new publisher factory
    pub fn new(event_bus: EventBus) -> Self {
        PublisherFactory { 
            event_bus,
            default_timeout: None,
        }
    }
    
    /// Create a new publisher factory with a default timeout
    pub fn with_timeout(event_bus: EventBus, timeout: Duration) -> Self {
        PublisherFactory { 
            event_bus,
            default_timeout: Some(timeout),
        }
    }
    
    /// Create a publisher for a specific event type
    pub fn create_publisher<E: Event>(&self) -> Publisher<E> {
        if let Some(timeout) = self.default_timeout {
            Publisher::with_timeout(self.event_bus.clone(), timeout)
        } else {
            Publisher::new(self.event_bus.clone())
        }
    }
    
    /// Create a fast publisher for a static event type
    pub fn create_fast_publisher<E: StaticEvent>(&self) -> FastPublisher<E> {
        FastPublisher::new()
    }
} 