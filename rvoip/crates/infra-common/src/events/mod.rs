/*!
Event System

This module provides a standardized event system for inter-component communication
in the RVOIP stack. It includes:

- Event bus for publishing and subscribing to events
- Strongly-typed event interfaces
- Support for both synchronous and asynchronous event handling
*/

pub mod bus;
pub mod publisher;
pub mod subscriber;
pub mod types;
pub mod registry;

pub use bus::EventBus;
pub use bus::EventBusConfig;
pub use publisher::Publisher;
pub use publisher::PublisherFactory;
pub use subscriber::SubscriberHandle;
pub use types::Event;
pub use types::EventPriority;
pub use types::EventFilter;
pub use types::StaticEvent;
pub use registry::TypeRegistry;
pub use registry::GlobalTypeRegistry; 