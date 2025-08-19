/*!
# infra-common

A common infrastructure layer for the RVOIP stack that provides:

- Event system for inter-component communication
- Configuration management
- Component lifecycle management
- Logging and metrics standardization
- Common error types and handling

This crate serves as a horizontal layer that all other components in the
RVOIP stack can leverage to ensure consistency and reduce duplication.
*/

// Set mimalloc as the global allocator for better memory performance
// Only when this crate is used as a binary, not as a library dependency
#[cfg(not(feature = "no-global-allocator"))]
#[global_allocator]
static GLOBAL: mimalloc::MiMalloc = mimalloc::MiMalloc;

pub mod events;
pub mod config;
pub mod lifecycle;
pub mod logging;
pub mod errors;
pub mod planes;

/// Re-export commonly used types
pub use events::bus::EventBus;
pub use events::bus::EventBusConfig;
pub use events::bus::EventPool;
pub use events::types::EventPriority;
pub use events::types::EventError;
pub use events::types::EventResult;
pub use events::bus::Publisher;
pub use events::bus::PublisherFactory;
pub use config::provider::ConfigProvider;
pub use lifecycle::component::Component;
pub use logging::setup::setup_logging;
pub use errors::types::Error;

/// Version information
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;
    use events::types::{Event, EventHandler, EventPriority};
    use async_trait::async_trait;
    use std::sync::Arc;
    use std::time::Duration;
    use tokio::runtime::Runtime;
    use serde::{Serialize, Deserialize};
    
    #[test]
    fn it_works() {
        // Basic test to verify crate builds
        assert_eq!(2 + 2, 4);
    }
    
    #[test]
    fn test_event_bus_creation() {
        let bus = events::bus::EventBus::new();
        assert!(bus.metrics().0 == 0, "New event bus should have 0 published events");
    }
    
    #[test]
    fn test_event_priorities() {
        assert!(events::types::EventPriority::Critical > events::types::EventPriority::High);
        assert!(events::types::EventPriority::High > events::types::EventPriority::Normal);
        assert!(events::types::EventPriority::Normal > events::types::EventPriority::Low);
    }
    
    // Sample event for testing
    #[derive(Clone, Debug, Serialize, Deserialize)]
    struct TestEvent {
        pub id: u64,
        pub data: String,
        pub priority: EventPriority,
    }

    impl Event for TestEvent {
        fn event_type() -> &'static str {
            "test_event"
        }

        fn priority() -> EventPriority {
            EventPriority::Normal
        }

        fn as_any(&self) -> &dyn std::any::Any {
            self
        }
    }

    struct TestHandler {
        counter: Arc<std::sync::atomic::AtomicU64>,
    }

    #[async_trait]
    impl EventHandler<TestEvent> for TestHandler {
        async fn handle(&self, _event: TestEvent) {
            // Simulate some work
            tokio::time::sleep(Duration::from_micros(10)).await;
            self.counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }

    fn create_test_event(id: u64, priority: EventPriority) -> TestEvent {
        TestEvent {
            id,
            data: format!("Event data for event {}", id),
            priority,
        }
    }
    
    #[test]
    fn test_event_bus_broadcast() {
        let rt = Runtime::new().unwrap();
        
        rt.block_on(async {
            // Setup
            let event_bus = events::bus::EventBus::new();
            let counter = Arc::new(std::sync::atomic::AtomicU64::new(0));
            
            println!("Registering direct subscriber...");
            // Register a direct subscriber
            let handler = TestHandler { counter: counter.clone() };
            let _ = event_bus.subscribe::<TestEvent, _>(None, handler).await.unwrap();
            
            println!("Creating broadcast channel...");
            // Also create a channel to receive events via broadcast
            let mut rx = event_bus.subscribe_broadcast::<TestEvent>().await.unwrap();
            
            // Spawn a task to count events received through the channel
            let channel_counter = Arc::new(std::sync::atomic::AtomicU64::new(0));
            let channel_counter_clone = channel_counter.clone();
            let channel_task = tokio::spawn(async move {
                println!("Channel receiver task started");
                while let Ok(event) = rx.recv().await {
                    println!("Received event on channel");
                    channel_counter_clone.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            });
            
            // Ensure we have time for all setup to complete
            tokio::time::sleep(Duration::from_millis(50)).await;
            
            println!("Publishing event...");
            // Send an event
            let result = event_bus.publish(create_test_event(1, EventPriority::Normal)).await;
            println!("Publish result: {:?}", result);
            
            // Wait for processing (longer to ensure completion)
            println!("Waiting for event processing...");
            tokio::time::sleep(Duration::from_millis(500)).await;
            
            // Verify the direct subscriber received the event
            let direct_count = counter.load(std::sync::atomic::Ordering::Relaxed);
            println!("Direct subscriber count: {}", direct_count);
            assert_eq!(direct_count, 1, "Direct subscriber should have received the event");
            
            // Verify the channel received the event
            let channel_count = channel_counter.load(std::sync::atomic::Ordering::Relaxed);
            println!("Channel subscriber count: {}", channel_count);
            assert_eq!(channel_count, 1, "Broadcast channel should have received the event");
            
            // Clean up
            channel_task.abort();
        });
    }
} 