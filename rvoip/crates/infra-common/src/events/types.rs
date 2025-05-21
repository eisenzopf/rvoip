use std::fmt::{self, Debug, Display};
use std::hash::Hash;
use std::sync::Arc;
use std::any::Any;
use async_trait::async_trait;
use std::time::Duration;
use serde::{Serialize, Deserialize};

/// Represents a type of event
pub type EventType = &'static str;

/// Priority levels for events
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash, Serialize, Deserialize)]
pub enum EventPriority {
    /// Low priority events
    Low = 0,
    /// Default priority events
    Normal = 1,
    /// High priority events
    High = 2,
    /// Critical events that must be processed immediately
    Critical = 3,
}

impl Default for EventPriority {
    fn default() -> Self {
        EventPriority::Normal
    }
}

/// Common trait for all events
pub trait Event: Clone + Send + Sync + Serialize + for<'de> Deserialize<'de> + 'static {
    /// Return the type identifier for this event
    fn event_type() -> EventType;
    
    /// Return the priority of this event
    fn priority() -> EventPriority {
        EventPriority::Normal
    }
    
    /// Convert to a typeless Any object (for internal use)
    fn as_any(&self) -> &dyn Any;
}

/// Trait for high-performance static events with cached type information
/// This provides fast paths for frequently used events
pub trait StaticEvent: Event {
    /// Get the cached type information
    fn static_type() -> EventType {
        Self::event_type()
    }
}

/// Predicate function for filtering events
pub type EventFilter<E> = Arc<dyn Fn(&E) -> bool + Send + Sync + 'static>;

/// Handler trait for processing events
#[async_trait]
pub trait EventHandler<E: Event>: Send + Sync {
    /// Process an event
    async fn handle(&self, event: E);
}

/// Implementation of EventHandler for closures
#[async_trait]
impl<E, F> EventHandler<E> for F
where
    E: Event,
    F: Fn(E) -> futures::future::BoxFuture<'static, ()> + Send + Sync + 'static,
{
    async fn handle(&self, event: E) {
        (self)(event).await;
    }
}

/// General error type for event operations
#[derive(Debug)]
pub enum EventError {
    /// Event subscription failed
    SubscriptionFailed(String),
    /// Event publishing failed
    PublishFailed(String),
    /// Invalid event type
    InvalidType(String),
    /// Handler timed out
    Timeout(String),
    /// Too many events in flight
    Overload(String),
    /// Invalid priority
    InvalidPriority(String),
    /// Event channel is closed or full
    ChannelError(String),
    /// Subscriber not found
    SubscriberNotFound(String),
    /// Other unspecified errors
    Other(String),
}

impl Display for EventError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> fmt::Result {
        match self {
            EventError::SubscriptionFailed(msg) => write!(f, "Subscription failed: {}", msg),
            EventError::PublishFailed(msg) => write!(f, "Failed to publish event: {}", msg),
            EventError::InvalidType(msg) => write!(f, "Invalid event type: {}", msg),
            EventError::Timeout(msg) => write!(f, "Event handler timed out: {}", msg),
            EventError::Overload(msg) => write!(f, "Event system overloaded: {}", msg),
            EventError::InvalidPriority(msg) => write!(f, "Invalid event priority: {}", msg),
            EventError::ChannelError(msg) => write!(f, "Event channel error: {}", msg),
            EventError::SubscriberNotFound(msg) => write!(f, "Subscriber not found: {}", msg),
            EventError::Other(msg) => write!(f, "Other error: {}", msg),
        }
    }
}

impl std::error::Error for EventError {}

/// Result type for event operations
pub type EventResult<T> = std::result::Result<T, EventError>; 