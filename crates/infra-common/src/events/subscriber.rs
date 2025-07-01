use crate::events::types::{Event, EventFilter, EventHandler, EventType, EventPriority};
use std::sync::Arc;
use std::any::Any;

use std::fmt::Debug;

/// Handle returned when subscribing to events
/// Can be used to unsubscribe
#[derive(Debug, Clone)]
pub struct SubscriberHandle {
    pub(crate) id: u64,
    pub(crate) event_type: EventType,
    pub(crate) priority: EventPriority,
}

/// Wrapper for event subscribers
#[derive(Clone)]
pub struct Subscriber {
    pub(crate) id: u64,
    pub(crate) event_type: EventType,
    pub(crate) priority: EventPriority,
    pub(crate) filter_fn: Arc<dyn Fn(&dyn Any) -> bool + Send + Sync + 'static>,
    pub(crate) handler_fn: Arc<dyn Fn(Box<dyn Any + Send + Sync>) -> futures::future::BoxFuture<'static, ()> + Send + Sync>,
}

impl Debug for Subscriber {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("Subscriber")
            .field("id", &self.id)
            .field("event_type", &self.event_type)
            .field("priority", &self.priority)
            .finish()
    }
}

impl Subscriber {
    /// Create a new subscriber
    pub fn new<E, H>(filter: Option<EventFilter<E>>, handler: H) -> Self
    where
        E: Event,
        H: EventHandler<E> + 'static,
    {
        static NEXT_ID: std::sync::atomic::AtomicU64 = std::sync::atomic::AtomicU64::new(1);
        let id = NEXT_ID.fetch_add(1, std::sync::atomic::Ordering::SeqCst);
        
        // Create type-erased filter function
        let filter_fn: Arc<dyn Fn(&dyn Any) -> bool + Send + Sync + 'static> = match filter {
            Some(f) => {
                let filter_fn = move |obj: &dyn Any| {
                    if let Some(event) = obj.downcast_ref::<E>() {
                        f(event)
                    } else {
                        false
                    }
                };
                Arc::new(filter_fn)
            },
            None => {
                let filter_fn = move |obj: &dyn Any| obj.downcast_ref::<E>().is_some();
                Arc::new(filter_fn)
            }
        };
        
        // Create type-erased handler
        let handler_clone = Arc::new(handler);
        let handler_fn = move |obj: Box<dyn Any + Send + Sync>| {
            let handler = handler_clone.clone();
            let future = async move {
                if let Ok(event) = obj.downcast::<E>() {
                    handler.handle(*event).await;
                }
            };
            Box::pin(future) as futures::future::BoxFuture<'static, ()>
        };
        
        Subscriber {
            id,
            event_type: E::event_type(),
            priority: E::priority(),
            filter_fn,
            handler_fn: Arc::new(handler_fn),
        }
    }
    
    /// Create a new subscriber with specific priority
    pub fn with_priority<E, H>(filter: Option<EventFilter<E>>, handler: H, priority: EventPriority) -> Self
    where
        E: Event,
        H: EventHandler<E> + 'static,
    {
        let mut subscriber = Self::new(filter, handler);
        subscriber.priority = priority;
        subscriber
    }
    
    /// Get a handle to this subscriber
    pub fn handle(&self) -> SubscriberHandle {
        SubscriberHandle {
            id: self.id,
            event_type: self.event_type,
            priority: self.priority,
        }
    }
    
    /// Check if this subscriber accepts a given event
    pub fn accepts_event<E: Event>(&self, event: &E) -> bool {
        (self.filter_fn)(event.as_any())
    }
    
    /// Handle an event
    pub async fn handle_event<E: Event>(&self, event: E) {
        let boxed = Box::new(event) as Box<dyn Any + Send + Sync>;
        let future = (self.handler_fn)(boxed);
        future.await;
    }
} 