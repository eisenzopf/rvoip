use crate::events::types::{Event, EventType};
use std::any::Any;
use std::sync::Arc;
use dashmap::DashMap;
use tokio::sync::broadcast;
use once_cell::sync::OnceCell;
use std::fmt::Debug;
use std::any::TypeId;
use std::sync::RwLock;
use std::sync::atomic::{AtomicUsize, Ordering};

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
}

impl<T> TypedBroadcastReceiver<T> {
    pub fn new(receiver: tokio::sync::broadcast::Receiver<Arc<T>>) -> Self {
        Self { receiver }
    }
    
    /// Receive a broadcast message.
    pub async fn recv(&mut self) -> Result<Arc<T>, tokio::sync::broadcast::error::RecvError> {
        self.receiver.recv().await
    }
    
    /// Try to receive a broadcast message without blocking.
    pub fn try_recv(&mut self) -> Result<Arc<T>, tokio::sync::broadcast::error::TryRecvError> {
        self.receiver.try_recv()
    }
    
    /// Resubscribe to a topic.
    pub fn resubscribe(&self) -> Self {
        Self { receiver: self.receiver.resubscribe() }
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
        let registry = GLOBAL_REGISTRY.get_or_init(|| TypeRegistry::new(Self::DEFAULT_CAPACITY));
        registry.get_or_create::<E>()
    }
    
    /// Subscribe to a specific static event type
    pub fn subscribe<E: Event + Send + Sync>() -> TypedBroadcastReceiver<E> {
        let registry = GLOBAL_REGISTRY.get_or_init(|| TypeRegistry::new(Self::DEFAULT_CAPACITY));
        let receiver = registry.get_or_create::<E>().subscribe();
        TypedBroadcastReceiver::new(receiver)
    }
    
    /// Register a specific event type with custom capacity
    pub fn register_with_capacity<E: Event + Send + Sync>(capacity: usize) -> TypedBroadcastSender<E> {
        let registry = GLOBAL_REGISTRY.get_or_init(|| TypeRegistry::new(Self::DEFAULT_CAPACITY));
        registry.register_with_capacity::<E>(capacity)
    }
    
    /// Register the default capacity for all channels
    pub fn register_default_capacity(capacity: usize) {
        let _registry = GLOBAL_REGISTRY.get_or_init(|| TypeRegistry::new(capacity));
        // Nothing else to do - the registry will use this capacity for future channels
    }
} 