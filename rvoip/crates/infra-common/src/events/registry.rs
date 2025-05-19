use crate::events::types::{Event, EventType};
use std::any::Any;
use std::sync::Arc;
use dashmap::DashMap;
use tokio::sync::broadcast;
use once_cell::sync::OnceCell;
use std::fmt::Debug;

/// Type-erased broadcast sender
pub trait AnyBroadcastSender: Send + Sync + Debug + Any {
    fn is_closed(&self) -> bool;
    fn receiver_count(&self) -> usize;
    fn as_any(&self) -> &dyn Any;
}

/// Typed broadcast sender for a specific event type
pub type TypedBroadcastSender<E> = broadcast::Sender<Arc<E>>;

/// Typed broadcast receiver for a specific event type
pub type TypedBroadcastReceiver<E> = broadcast::Receiver<Arc<E>>;

impl<E: Event> AnyBroadcastSender for TypedBroadcastSender<E> {
    fn is_closed(&self) -> bool {
        self.receiver_count() == 0
    }
    
    fn receiver_count(&self) -> usize {
        broadcast::Sender::receiver_count(self)
    }
    
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Registry for type-specific broadcast channels
#[derive(Debug)]
pub struct TypeRegistry {
    /// Map from event type to type-erased broadcast sender
    channels: DashMap<EventType, Box<dyn AnyBroadcastSender>>,
    /// Default channel capacity
    capacity: usize,
}

impl TypeRegistry {
    /// Create a new type registry with the given channel capacity
    pub fn new(capacity: usize) -> Self {
        Self {
            channels: DashMap::new(),
            capacity,
        }
    }
    
    /// Register a new event type with a broadcast channel
    pub fn register<E: Event>(&self) -> TypedBroadcastSender<E> {
        self.register_with_capacity::<E>(self.capacity)
    }
    
    /// Register a new event type with a specific capacity
    pub fn register_with_capacity<E: Event>(&self, capacity: usize) -> TypedBroadcastSender<E> {
        let event_type = E::event_type();
        
        // If it already exists, try to return the existing one
        if let Some(entry) = self.channels.get(event_type) {
            // Try to downcast to the specific sender type
            let sender = entry.value();
            if let Some(typed_sender) = sender.as_any().downcast_ref::<TypedBroadcastSender<E>>() {
                return typed_sender.clone();
            }
            // Wrong type - this is a safety error, we'll replace it
            tracing::error!("Type registry contains wrong type for {}", event_type);
        }
        
        // Create a new broadcast channel
        let (tx, _) = broadcast::channel::<Arc<E>>(capacity);
        let typed_tx = tx.clone();
        self.channels.insert(event_type, Box::new(tx));
        typed_tx
    }
    
    /// Get or create a broadcast sender for a specific event type
    pub fn get_or_create<E: Event>(&self) -> TypedBroadcastSender<E> {
        if let Some(sender) = self.get::<E>() {
            return sender;
        }
        self.register::<E>()
    }
    
    /// Get an existing broadcast sender for a specific event type
    pub fn get<E: Event>(&self) -> Option<TypedBroadcastSender<E>> {
        let event_type = E::event_type();
        if let Some(entry) = self.channels.get(event_type) {
            // Downcast to the specific sender type
            if let Some(typed_sender) = entry.value().as_any().downcast_ref::<TypedBroadcastSender<E>>() {
                return Some(typed_sender.clone());
            }
        }
        None
    }
    
    /// Remove a broadcast channel for a specific event type
    pub fn remove<E: Event>(&self) -> Option<TypedBroadcastSender<E>> {
        let event_type = E::event_type();
        if let Some((_, boxed)) = self.channels.remove(event_type) {
            // Downcast to the specific sender type
            if let Some(sender) = boxed.as_any().downcast_ref::<TypedBroadcastSender<E>>() {
                return Some(sender.clone());
            }
        }
        None
    }
    
    /// Check if a broadcast channel exists for a specific event type
    pub fn contains<E: Event>(&self) -> bool {
        let event_type = E::event_type();
        self.channels.contains_key(event_type)
    }
    
    /// Get the number of registered event types
    pub fn len(&self) -> usize {
        self.channels.len()
    }
    
    /// Check if there are any registered event types
    pub fn is_empty(&self) -> bool {
        self.channels.is_empty()
    }
    
    /// Create a new broadcast receiver for a specific event type
    pub fn subscribe<E: Event>(&self) -> Option<TypedBroadcastReceiver<E>> {
        self.get::<E>().map(|sender| sender.subscribe())
    }
    
    /// Create a new broadcast receiver or register if not exists
    pub fn subscribe_or_create<E: Event>(&self) -> TypedBroadcastReceiver<E> {
        self.get_or_create::<E>().subscribe()
    }
}

/// Global type registry for static event types
#[derive(Debug)]
pub struct GlobalTypeRegistry;

impl GlobalTypeRegistry {
    /// Default channel capacity optimized for high throughput
    const DEFAULT_CAPACITY: usize = 16384;
    
    /// Get or create a broadcast sender for a specific static event type
    pub fn get_sender<E: Event>() -> TypedBroadcastSender<E> {
        static REGISTRY: OnceCell<TypeRegistry> = OnceCell::new();
        let registry = REGISTRY.get_or_init(|| TypeRegistry::new(Self::DEFAULT_CAPACITY));
        registry.get_or_create::<E>()
    }
    
    /// Subscribe to a specific static event type
    pub fn subscribe<E: Event>() -> TypedBroadcastReceiver<E> {
        Self::get_sender::<E>().subscribe()
    }
    
    /// Register a specific event type with custom capacity
    pub fn register_with_capacity<E: Event>(capacity: usize) -> TypedBroadcastSender<E> {
        static REGISTRY: OnceCell<TypeRegistry> = OnceCell::new();
        let registry = REGISTRY.get_or_init(|| TypeRegistry::new(Self::DEFAULT_CAPACITY));
        registry.register_with_capacity::<E>(capacity)
    }
} 