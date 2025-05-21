use crate::events::types::{Event, EventType, StaticEvent};
use std::any::Any;
use std::sync::Arc;
use dashmap::DashMap;
use tokio::sync::broadcast;
use once_cell::sync::OnceCell;
use std::fmt::Debug;
use std::any::TypeId;
use std::sync::RwLock;
use std::sync::atomic::{AtomicUsize, Ordering};
use tracing;

// Static event registry to properly track types implementing StaticEvent
static STATIC_EVENT_REGISTRY: once_cell::sync::Lazy<dashmap::DashSet<TypeId>> = 
    once_cell::sync::Lazy::new(|| dashmap::DashSet::new());

/// Register a type as a StaticEvent in the registry
pub fn register_static_event<T: 'static + StaticEvent>() {
    STATIC_EVENT_REGISTRY.insert(TypeId::of::<T>());
}

/// Check if a type is registered as a StaticEvent
pub fn is_registered_static_event<T: 'static>() -> bool {
    STATIC_EVENT_REGISTRY.contains(&TypeId::of::<T>())
}

/// Helper function to check if a type is registered as a StaticEvent
/// This is used since std::any::any_is is not available
fn is_type_static_event<T: 'static>() -> bool {
    // Check if T is registered in our static event registry
    is_registered_static_event::<T>()
}

/// Trait for erased broadcast senders of any type.
trait AnyBroadcastSender: Send + Sync + 'static {
    fn as_any(&self) -> &dyn Any;
    fn type_name(&self) -> &str;
    fn clone_sender(&self) -> Box<dyn AnyBroadcastSender>;
    fn fmt_debug(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result;
}

/// Implementation of AnyBroadcastSender for a specific event type.
struct AnyBroadcastSenderImpl<T: 'static + Send + Sync> {
    sender: TypedBroadcastSender<T>,
    type_name: String,
}

impl<T: 'static + Send + Sync> AnyBroadcastSender for AnyBroadcastSenderImpl<T> {
    fn as_any(&self) -> &dyn Any {
        &self.sender
    }
    
    fn type_name(&self) -> &str {
        &self.type_name
    }
    
    fn clone_sender(&self) -> Box<dyn AnyBroadcastSender> {
        Box::new(AnyBroadcastSenderImpl {
            sender: self.sender.clone(),
            type_name: self.type_name.clone(),
        })
    }
    
    fn fmt_debug(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "AnyBroadcastSender(type={}, receiver_count={})", self.type_name, self.sender.receiver_count())
    }
}

/// Debug implementation for Box<dyn AnyBroadcastSender>
impl Debug for Box<dyn AnyBroadcastSender> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.as_ref().fmt_debug(f)
    }
}

/// Typed broadcast sender for a specific event type
pub type TypedBroadcastSender<E> = broadcast::Sender<Arc<E>>;

/// Typed wrapper around tokio::sync::broadcast::Receiver for events of a specific type
pub struct TypedBroadcastReceiver<T> {
    pub(crate) receiver: tokio::sync::broadcast::Receiver<Arc<T>>,
    type_name: String,
}

impl<T> TypedBroadcastReceiver<T> {
    pub fn new(receiver: tokio::sync::broadcast::Receiver<Arc<T>>) -> Self {
        let type_name = std::any::type_name::<T>().to_string();
        tracing::debug!("Created TypedBroadcastReceiver for {}", type_name);
        Self { 
            receiver,
            type_name,
        }
    }
    
    /// Receive a broadcast message.
    pub async fn recv(&mut self) -> Result<Arc<T>, tokio::sync::broadcast::error::RecvError> {
        match self.receiver.recv().await {
            Ok(event) => {
                tracing::trace!("TypedBroadcastReceiver received event for {}", self.type_name);
                Ok(event)
            },
            Err(e) => {
                tracing::warn!("TypedBroadcastReceiver error for {}: {}", self.type_name, e);
                Err(e)
            }
        }
    }
    
    /// Try to receive a broadcast message without blocking.
    pub fn try_recv(&mut self) -> Result<Arc<T>, tokio::sync::broadcast::error::TryRecvError> {
        match self.receiver.try_recv() {
            Ok(event) => {
                tracing::trace!("TypedBroadcastReceiver try_recv successful for {}", self.type_name);
                Ok(event)
            },
            Err(e) => {
                // Only log warnings for actual errors, not for Empty (which is normal)
                if !matches!(e, tokio::sync::broadcast::error::TryRecvError::Empty) {
                    tracing::warn!("TypedBroadcastReceiver try_recv error for {}: {}", self.type_name, e);
                }
                Err(e)
            }
        }
    }
    
    /// Resubscribe to a topic.
    pub fn resubscribe(&self) -> Self {
        tracing::debug!("Resubscribing TypedBroadcastReceiver for {}", self.type_name);
        Self { 
            receiver: self.receiver.resubscribe(),
            type_name: self.type_name.clone(),
        }
    }
}

/// A type registry for managing broadcast channels of various event types.
pub struct TypeRegistry {
    default_capacity: AtomicUsize,
    senders: DashMap<TypeId, Box<dyn AnyBroadcastSender>>,
}

/// Custom Debug implementation for TypeRegistry
impl Debug for TypeRegistry {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("TypeRegistry")
            .field("default_capacity", &self.default_capacity.load(Ordering::Relaxed))
            .field("sender_count", &self.senders.len())
            .finish()
    }
}

impl TypeRegistry {
    /// Create a new TypeRegistry with default capacity for all channels
    pub fn new(default_capacity: usize) -> Self {
        Self {
            default_capacity: AtomicUsize::new(default_capacity),
            senders: DashMap::new(),
        }
    }
    
    /// Register a type with the registry, creating a sender if it doesn't exist
    pub fn register<E: Event + Send + Sync>(&self) -> TypedBroadcastSender<E> {
        self.get_or_create::<E>()
    }
    
    /// Set the default capacity for new channels
    pub fn set_default_capacity(&self, capacity: usize) {
        self.default_capacity.store(capacity, Ordering::Relaxed);
    }
    
    /// Get the registered sender for a type, if it exists
    pub fn get<E: Event + Send + Sync>(&self) -> Option<TypedBroadcastSender<E>> {
        let key = std::any::TypeId::of::<E>();
        self.senders.get(&key).map(|sender| {
            // Directly clone the sender from the any sender
            // This is safer than transmute as we know the type matches by TypeId
            let any_sender = sender.as_any();
            let typed_sender = any_sender.downcast_ref::<TypedBroadcastSender<E>>()
                .expect("Type mismatch in registry");
            typed_sender.clone()
        })
    }
    
    /// Get or create a sender for a type
    pub fn get_or_create<E: Event + Send + Sync>(&self) -> TypedBroadcastSender<E> {
        let key = std::any::TypeId::of::<E>();
        
        // Check if we already have a sender for this type
        if let Some(sender) = self.senders.get(&key) {
            let any_sender = sender.as_any();
            let typed_sender = any_sender.downcast_ref::<TypedBroadcastSender<E>>()
                .expect("Type mismatch in registry");
            return typed_sender.clone();
        }
        
        // Create a new sender
        let capacity = self.default_capacity.load(Ordering::Relaxed);
        let (sender, _) = broadcast::channel::<Arc<E>>(capacity);
        let any_sender = AnyBroadcastSenderImpl {
            sender: sender.clone(),
            type_name: std::any::type_name::<E>().to_string(),
        };
        
        // Store the sender
        self.senders.insert(key, Box::new(any_sender));
        
        sender
    }
    
    /// Register a new sender with custom capacity
    pub fn register_with_capacity<E: Event + Send + Sync>(&self, capacity: usize) -> TypedBroadcastSender<E> {
        let key = std::any::TypeId::of::<E>();
        
        // Create a new sender with the specified capacity
        let (sender, _) = broadcast::channel::<Arc<E>>(capacity);
        let any_sender = AnyBroadcastSenderImpl {
            sender: sender.clone(),
            type_name: std::any::type_name::<E>().to_string(),
        };
        
        // Store the sender, replacing any existing one
        self.senders.insert(key, Box::new(any_sender));
        
        sender
    }
    
    /// Subscribe to a type if it exists
    pub fn subscribe<E: Event + Send + Sync>(&self) -> Option<TypedBroadcastReceiver<E>> {
        self.get::<E>().map(|sender| TypedBroadcastReceiver::new(sender.subscribe()))
    }
    
    /// Subscribe to a type, creating it if it doesn't exist
    pub fn subscribe_or_create<E: Event + Send + Sync>(&self) -> TypedBroadcastReceiver<E> {
        let sender = self.get_or_create::<E>();
        TypedBroadcastReceiver::new(sender.subscribe())
    }
}

/// Global type registry singleton
pub struct GlobalTypeRegistry;

// Use OnceCell instead of lazy_static
static GLOBAL_REGISTRY: once_cell::sync::OnceCell<TypeRegistry> = once_cell::sync::OnceCell::new();

impl GlobalTypeRegistry {
    /// Default channel capacity optimized for high throughput
    const DEFAULT_CAPACITY: usize = 16384;
    
    /// Get or create a broadcast sender for a specific static event type
    pub fn get_sender<E: Event + Send + Sync>() -> TypedBroadcastSender<E> {
        let registry = GLOBAL_REGISTRY.get_or_init(|| {
            tracing::info!("Initializing GlobalTypeRegistry with default capacity {}", Self::DEFAULT_CAPACITY);
            TypeRegistry::new(Self::DEFAULT_CAPACITY)
        });
        registry.get_or_create::<E>()
    }
    
    /// Subscribe to a specific static event type
    pub fn subscribe<E: Event + Send + Sync>() -> TypedBroadcastReceiver<E> {
        let registry = GLOBAL_REGISTRY.get_or_init(|| {
            tracing::info!("Initializing GlobalTypeRegistry with default capacity {}", Self::DEFAULT_CAPACITY);
            TypeRegistry::new(Self::DEFAULT_CAPACITY)
        });
        
        // Make sure we've registered this type
        if is_registered_static_event::<E>() {
            tracing::debug!("Using pre-registered StaticEvent channel for {}", std::any::type_name::<E>());
        } else {
            // Warn if not using built-in test events
            tracing::warn!("Subscribing to {} which is not registered as StaticEvent", std::any::type_name::<E>());
        }
        
        // Get or create the sender
        let sender = registry.get_or_create::<E>();
        
        // Create and return the receiver
        let receiver = sender.subscribe();
        TypedBroadcastReceiver::new(receiver)
    }
    
    /// Register a specific event type with custom capacity
    pub fn register_with_capacity<E: Event + Send + Sync>(capacity: usize) -> TypedBroadcastSender<E> {
        let registry = GLOBAL_REGISTRY.get_or_init(|| {
            tracing::info!("Initializing GlobalTypeRegistry with default capacity {}", Self::DEFAULT_CAPACITY);
            TypeRegistry::new(Self::DEFAULT_CAPACITY)
        });
        
        // Register as static event if it's in our registry
        if is_type_static_event::<E>() {
            // This already means E is registered, but we'll make sure
            STATIC_EVENT_REGISTRY.insert(TypeId::of::<E>());
            tracing::debug!("Registered {} as StaticEvent with capacity {}", std::any::type_name::<E>(), capacity);
        }
        
        registry.register_with_capacity::<E>(capacity)
    }
    
    /// Register the default capacity for all channels
    pub fn register_default_capacity(capacity: usize) {
        let registry = GLOBAL_REGISTRY.get_or_init(|| {
            tracing::info!("Initializing GlobalTypeRegistry with capacity {}", capacity);
            TypeRegistry::new(capacity)
        });
        
        registry.set_default_capacity(capacity);
        tracing::debug!("Set default channel capacity to {}", capacity);
    }
    
    /// Register a type as implementing StaticEvent
    pub fn register_static_event_type<E: StaticEvent>() {
        register_static_event::<E>();
        tracing::debug!("Registered {} as StaticEvent in global registry", std::any::type_name::<E>());
    }
    
    /// Check if a type is registered as implementing StaticEvent
    pub fn is_static_event<E: 'static>() -> bool {
        let is_registered = is_registered_static_event::<E>();
        tracing::trace!("Checking if {} is registered as StaticEvent: {}", std::any::type_name::<E>(), is_registered);
        is_registered
    }
} 