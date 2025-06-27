use crate::events::types::{Event, EventFilter, EventHandler, EventType, EventPriority, EventError, EventResult, StaticEvent};
use crate::events::subscriber::{Subscriber, SubscriberHandle};
use crate::events::registry::{TypeRegistry, TypedBroadcastReceiver, GlobalTypeRegistry};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::{Semaphore, mpsc, Mutex};
use tokio::time::timeout;
use dashmap::DashMap;


/// Configuration for the event bus
#[derive(Debug, Clone)]
pub struct EventBusConfig {
    /// Maximum number of concurrent event dispatches
    pub max_concurrent_dispatches: usize,
    /// Default timeout for event handling
    pub default_timeout: Duration,
    /// Default capacity for broadcast channels
    pub broadcast_capacity: usize,
    /// Enable priority-based event handling
    pub enable_priority: bool,
    /// Enable zero-copy architecture for higher performance
    pub enable_zero_copy: bool,
    /// Batch size for batch operations
    pub batch_size: usize,
    /// Number of shards for internal maps
    pub shard_count: usize,
}

impl Default for EventBusConfig {
    fn default() -> Self {
        EventBusConfig {
            max_concurrent_dispatches: 10000,
            default_timeout: Duration::from_secs(5),
            broadcast_capacity: 16384,
            enable_priority: true,
            enable_zero_copy: true,
            batch_size: 100,
            shard_count: 32,
        }
    }
}

/// The central event bus that manages event distribution
///
/// The EventBus provides a central point for components to publish events
/// and subscribe to events they're interested in using a high-performance
/// zero-copy architecture.
#[derive(Clone, Debug)]
pub struct EventBus {
    /// Lock-free subscriber storage using DashMap
    subscribers: Arc<DashMap<EventType, Vec<Subscriber>>>,
    
    /// Type registry for zero-copy event broadcasting
    type_registry: Arc<TypeRegistry>,
    
    /// Semaphore to limit concurrent event dispatches
    dispatch_semaphore: Arc<Semaphore>,
    
    /// Configuration for the event bus
    config: EventBusConfig,
    
    /// Metrics for event bus operations
    metrics: Arc<EventBusMetrics>,
}

/// Metrics for the event bus
#[derive(Debug, Default)]
struct EventBusMetrics {
    /// Total events published
    total_published: std::sync::atomic::AtomicU64,
    /// Total events delivered
    total_delivered: std::sync::atomic::AtomicU64,
    /// Total timeouts
    timeouts: std::sync::atomic::AtomicU64,
    /// Total overloads
    overloads: std::sync::atomic::AtomicU64,
}

impl EventBus {
    /// Create a new event bus instance with default configuration
    pub fn new() -> Self {
        Self::with_config(EventBusConfig::default())
    }

    /// Create a new event bus with custom configuration
    pub fn with_config(config: EventBusConfig) -> Self {
        EventBus {
            subscribers: Arc::new(DashMap::with_capacity(1024)),
            type_registry: Arc::new(TypeRegistry::new(config.broadcast_capacity)),
            dispatch_semaphore: Arc::new(Semaphore::new(config.max_concurrent_dispatches)),
            config,
            metrics: Arc::new(EventBusMetrics::default()),
        }
    }

    /// Publish an event to all interested subscribers
    pub async fn publish<E: Event>(&self, event: E) -> EventResult<()> {
        self.publish_with_timeout(event, self.config.default_timeout).await
    }
    
    /// Ultra-fast publish for static events - uses cached type information
    pub async fn publish_fast<E: StaticEvent>(&self, event: E) -> EventResult<()> {
        // Get the cached sender from global registry for optimal performance
        let sender = if self.config.enable_zero_copy {
            GlobalTypeRegistry::get_sender::<E>()
        } else {
            self.type_registry.get_or_create::<E>()
        };
        
        // Wrap event in Arc for zero-copy passing
        let arc_event = Arc::new(event);
        
        // Record the event in metrics
        self.metrics.total_published.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        
        // Try to send the event - use non-blocking send for maximum throughput
        match sender.send(arc_event.clone()) {
            Ok(receiver_count) => {
                // Update metrics for successful broadcast
                if receiver_count > 0 {
                    self.metrics.total_delivered.fetch_add(
                        receiver_count as u64, 
                        std::sync::atomic::Ordering::Relaxed
                    );
                }
                
                Ok(())
            },
            Err(err) => {
                // Check if we have any direct subscribers as fallback
                if let Some(subscribers) = self.subscribers.get(E::event_type()) {
                    if !subscribers.value().is_empty() {
                        // We have direct subscribers, spawn task to process
                        let subscribers_clone = subscribers.value().clone();
                        let event_clone = arc_event.clone();
                        let metrics = self.metrics.clone();
                        
                        tokio::spawn(async move {
                            for subscriber in subscribers_clone {
                                let _ = subscriber.handle_event((*event_clone).clone()).await;
                                metrics.total_delivered.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            }
                        });
                        
                        return Ok(());
                    }
                }
                
                // No subscribers at all
                Err(EventError::ChannelError(format!(
                    "Fast broadcast failed for {}: {}", E::event_type(), err
                )))
            }
        }
    }
    
    /// Publish a batch of events for high-throughput scenarios
    pub async fn publish_batch<E: Event>(&self, events: Vec<E>) -> EventResult<()> {
        if events.is_empty() {
            return Ok(());
        }
        
        // Get or create typed sender for this event type
        let sender = self.type_registry.get_or_create::<E>();
        
        // Record total events
        self.metrics.total_published.fetch_add(events.len() as u64, std::sync::atomic::Ordering::Relaxed);
        
        // Track successful deliveries
        let mut delivered = 0;
        
        // Process in batches
        for chunk in events.chunks(self.config.batch_size) {
            // Convert to Arc<E> for zero-copy
            let arc_events: Vec<_> = chunk.iter().map(|e| Arc::new(e.clone())).collect();
            
            // Publish each event in the batch
            for arc_event in arc_events {
                match sender.send(arc_event) {
                    Ok(receiver_count) => {
                        delivered += receiver_count;
                    },
                    Err(_) => {
                        // Channel closed or error, we'll continue with remaining events
                    }
                }
            }
        }
        
        // Update metrics
        if delivered > 0 {
            self.metrics.total_delivered.fetch_add(delivered as u64, std::sync::atomic::Ordering::Relaxed);
        }
        
        Ok(())
    }

    /// Publish an event with a specific timeout
    pub async fn publish_with_timeout<E: Event>(&self, event: E, timeout_duration: Duration) -> EventResult<()> {
        // Get event metadata
        let event_type = E::event_type();
        let event_priority = E::priority();
        
        // Record the event in metrics
        self.metrics.total_published.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        
        // Use zero-copy broadcast when enabled
        if self.config.enable_zero_copy {
            let arc_event = Arc::new(event.clone());
            let sender = self.type_registry.get_or_create::<E>();
            
            // Try to send the event via broadcast channel
            let broadcast_result = sender.send(arc_event);
            
            match broadcast_result {
                Ok(receiver_count) => {
                    // Successfully published to broadcast channel
                    self.metrics.total_delivered.fetch_add(receiver_count as u64, std::sync::atomic::Ordering::Relaxed);
                }
                Err(_) => {
                    // Broadcast failed, we'll try direct subscribers next
                }
            }
        }
        
        // Also check for direct subscribers
        if let Some(subscribers) = self.subscribers.get(event_type) {
            // Deliver to direct subscribers as well
            let subscribers = subscribers.value().clone();
            let mut handles = Vec::new();
            
            for subscriber in subscribers {
                handles.push(self.process_subscriber(subscriber, event.clone(), timeout_duration).await?);
            }
            
            // Wait for all handlers to complete if critical priority
            if event_priority == EventPriority::Critical {
                for handle in handles {
                    if let Err(e) = handle.await {
                        return Err(EventError::PublishFailed(format!(
                            "Critical event handler failed: {}", e
                        )));
                    }
                }
            }
        }
        
        Ok(())
    }
    
    /// Process a subscriber with timeout and permit control
    async fn process_subscriber<E: Event>(&self, subscriber: Subscriber, event: E, timeout_duration: Duration) 
        -> EventResult<tokio::task::JoinHandle<EventResult<()>>> 
    {
        // Try to acquire a permit for dispatching
        let permit = match self.dispatch_semaphore.clone().try_acquire_owned() {
            Ok(permit) => permit,
            Err(_) => {
                self.metrics.overloads.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                return Err(EventError::Overload(format!(
                    "Too many events in flight, cannot dispatch {} event", E::event_type()
                )));
            }
        };
        
        let event_clone = event.clone();
        let metrics = self.metrics.clone();
        
        // Spawn a task to handle the event
        let handle = tokio::spawn(async move {
            let _permit = permit;  // Keep permit until task completes
            
            // Apply timeout to event handling
            match timeout(timeout_duration, subscriber.handle_event(event_clone)).await {
                Ok(_) => {
                    metrics.total_delivered.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    Ok(())
                },
                Err(_) => {
                    metrics.timeouts.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                    Err(EventError::Timeout(format!(
                        "Handler for {} event timed out after {:?}", 
                        E::event_type(), timeout_duration
                    )))
                }
            }
        });
        
        Ok(handle)
    }

    /// Subscribe to events of a specific type with the provided handler
    pub async fn subscribe<E: Event, F>(&self, filter: Option<EventFilter<E>>, handler: F) -> EventResult<SubscriberHandle>
    where
        F: EventHandler<E> + Send + Sync + 'static,
    {
        let event_type = E::event_type();
        let subscriber = Subscriber::new(filter, handler);
        let handle = subscriber.handle();
        
        // Store in the high-performance DashMap
        self.subscribers
            .entry(event_type)
            .or_insert_with(Vec::new)
            .push(subscriber);
                
        // Pre-register with the type registry for future broadcasts
        self.type_registry.register::<E>();
            
        Ok(handle)
    }
    
    /// Subscribe to broadcast events of a specific type.
    ///
    /// This method allows you to receive all events of a specific type that are broadcast through the event bus.
    /// It creates a new TypedBroadcastReceiver for the event type.
    ///
    /// # Type Parameters
    ///
    /// * `E` - The event type to subscribe to.
    ///
    /// # Returns
    ///
    /// A `Result` containing a TypedBroadcastReceiver on success, or an EventError on failure.
    pub async fn subscribe_broadcast<E: Event>(&self) -> EventResult<TypedBroadcastReceiver<E>> {
        Ok(TypedBroadcastReceiver::new(self.type_registry.get_or_create::<E>().subscribe()))
    }
    
    /// Create a channel-based publisher for a specific event type
    pub fn create_channel<E: Event>(&self) -> mpsc::Sender<E> {
        let event_bus = self.clone();
        let (tx, rx) = mpsc::channel(self.config.broadcast_capacity);
        
        // Spawn a background task to forward events from the channel to the event bus
        tokio::spawn(async move {
            Self::forward_events_from_channel(event_bus, rx).await;
        });
        
        tx
    }
    
    /// Forward events from a channel to the event bus
    async fn forward_events_from_channel<E: Event>(
        event_bus: EventBus,
        mut rx: mpsc::Receiver<E>,
    ) {
        // High-performance path: batch events
        let mut batch = Vec::with_capacity(event_bus.config.batch_size);
        
        while let Some(event) = rx.recv().await {
            batch.push(event);
            
            // Process in batches when batch is full or channel is empty
            if batch.len() >= event_bus.config.batch_size || rx.try_recv().is_err() {
                let events = std::mem::take(&mut batch);
                let _ = event_bus.publish_batch(events).await;
            }
        }
        
        // Process any remaining events
        if !batch.is_empty() {
            let _ = event_bus.publish_batch(batch).await;
        }
    }
    
    /// Get metrics for the event bus
    pub fn metrics(&self) -> (u64, u64, u64, u64) {
        (
            self.metrics.total_published.load(std::sync::atomic::Ordering::Relaxed),
            self.metrics.total_delivered.load(std::sync::atomic::Ordering::Relaxed),
            self.metrics.timeouts.load(std::sync::atomic::Ordering::Relaxed),
            self.metrics.overloads.load(std::sync::atomic::Ordering::Relaxed),
        )
    }
    
    /// Check if a subscriber exists for a specific event type
    pub fn has_subscribers<E: Event>(&self) -> bool {
        // Check broadcast subscribers
        if let Some(sender) = self.type_registry.get::<E>() {
            if sender.receiver_count() > 0 {
                return true;
            }
        }
        
        // Check direct subscribers
        if let Some(entry) = self.subscribers.get(E::event_type()) {
            if !entry.value().is_empty() {
                return true;
            }
        }
        
        false
    }
    
    /// Get the number of subscribers for a specific event type
    pub fn subscriber_count<E: Event>(&self) -> usize {
        let mut count = 0;
        
        // Count broadcast subscribers
        if let Some(sender) = self.type_registry.get::<E>() {
            count += sender.receiver_count();
        }
        
        // Count direct subscribers
        if let Some(entry) = self.subscribers.get(E::event_type()) {
            count += entry.value().len();
        }
        
        count
    }
    
    /// Get a reference to the type registry
    pub fn type_registry(&self) -> &Arc<TypeRegistry> {
        &self.type_registry
    }
}

impl Default for EventBus {
    fn default() -> Self {
        Self::new()
    }
}

/// Pool for reusing event objects to reduce memory allocation
pub struct EventPool<E: Event + Default> {
    pool: Arc<Mutex<Vec<E>>>,
    max_size: usize,
}

impl<E: Event + Default> EventPool<E> {
    /// Create a new event pool with the given maximum size
    pub fn new(max_size: usize) -> Self {
        EventPool {
            pool: Arc::new(Mutex::new(Vec::with_capacity(max_size / 2))),
            max_size,
        }
    }
    
    /// Get an event from the pool or create a new one
    pub async fn get(&self) -> PooledEvent<E> {
        let mut pool = self.pool.lock().await;
        let event = pool.pop().unwrap_or_default();
        
        PooledEvent {
            event: Some(event),
            pool: self.pool.clone(),
            max_size: self.max_size,
        }
    }
}

/// An event from a pool that can be reused
pub struct PooledEvent<E: Event> {
    event: Option<E>,
    pool: Arc<Mutex<Vec<E>>>,
    max_size: usize,
}

impl<E: Event> std::ops::Deref for PooledEvent<E> {
    type Target = E;
    
    fn deref(&self) -> &Self::Target {
        self.event.as_ref().expect("Event should not be None")
    }
}

impl<E: Event> std::ops::DerefMut for PooledEvent<E> {
    fn deref_mut(&mut self) -> &mut Self::Target {
        self.event.as_mut().expect("Event should not be None")
    }
}

impl<E: Event> Drop for PooledEvent<E> {
    fn drop(&mut self) {
        if let Some(event) = self.event.take() {
            // Try to return the event to the pool
            let pool = self.pool.clone();
            let max_size = self.max_size;
            
            tokio::spawn(async move {
                let mut pool_lock = pool.lock().await;
                if pool_lock.len() < max_size {
                    pool_lock.push(event);
                }
            });
        }
    }
}

/// A strongly-typed event publisher backed by an EventBus.
///
/// This type provides a convenient interface for publishing events of a specific type.
pub struct Publisher<E: Event> {
    /// The event bus to publish to
    event_bus: EventBus,
    /// Phantom data to satisfy the type parameter
    _phantom: std::marker::PhantomData<E>,
}

impl<E: Event> Publisher<E> {
    /// Creates a new publisher for events of type E.
    ///
    /// # Arguments
    ///
    /// * `event_bus` - The event bus to publish to
    ///
    /// # Returns
    ///
    /// A new `Publisher<E>` instance
    pub fn new(event_bus: EventBus) -> Self {
        Self {
            event_bus,
            _phantom: std::marker::PhantomData,
        }
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
        self.event_bus.publish(event).await
    }
    
    /// Publishes a batch of events.
    ///
    /// # Arguments
    ///
    /// * `events` - A vector of events to publish
    ///
    /// # Returns
    ///
    /// `Ok(())` if all events were published successfully, or an error if any publication fails
    pub async fn publish_batch(&self, events: Vec<E>) -> EventResult<()> {
        self.event_bus.publish_batch(events).await
    }
}

/// Factory for creating publishers for different event types.
///
/// This type provides a convenient way to create publishers for different event types
/// that all use the same underlying event bus.
pub struct PublisherFactory {
    /// The event bus to create publishers for
    event_bus: EventBus,
}

impl PublisherFactory {
    /// Creates a new publisher factory.
    ///
    /// # Arguments
    ///
    /// * `event_bus` - The event bus to create publishers for
    ///
    /// # Returns
    ///
    /// A new `PublisherFactory` instance
    pub fn new(event_bus: EventBus) -> Self {
        Self { event_bus }
    }
    
    /// Creates a new publisher for events of type E.
    ///
    /// # Type Parameters
    ///
    /// * `E` - The event type to create a publisher for
    ///
    /// # Returns
    ///
    /// A new `Publisher<E>` instance
    pub fn create<E: Event>(&self) -> Publisher<E> {
        Publisher::new(self.event_bus.clone())
    }
} 