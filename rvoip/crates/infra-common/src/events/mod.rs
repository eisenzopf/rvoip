/*!
Event System

This module provides a standardized event system for inter-component communication
in the RVOIP stack. It includes:

- Event bus for publishing and subscribing to events
- Strongly-typed event interfaces
- Support for both synchronous and asynchronous event handling
- Unified API for working with different event system implementations
*/

pub mod bus;
pub mod publisher;
pub mod registry;
pub mod subscriber;
pub mod types;
pub mod system;
pub mod builder;

pub use bus::EventBus;
pub use bus::EventBusConfig;
pub use publisher::{Publisher, FastPublisher, PublisherFactory};
pub use subscriber::Subscriber;
pub use types::{Event, StaticEvent, EventPriority, EventError, EventResult};
pub use system::{EventSystem, EventPublisher, EventSubscriber};
pub use builder::{EventSystemBuilder, ImplementationType, BackpressureStrategy, BackpressureAction}; 