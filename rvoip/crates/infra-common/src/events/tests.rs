use crate::events::api::{EventSystem, EventPublisher, EventSubscriber};
use crate::events::builder::{EventSystemBuilder, ImplementationType};
use crate::events::types::{Event, EventPriority, StaticEvent};
use serde::{Serialize, Deserialize};
use std::any::Any;
use std::sync::Arc;
use std::time::Duration;

#[cfg(test)]
mod tests {
    use super::*;
    
    /// Test event that can be used with both implementations
    #[derive(Clone, Debug, Serialize, Deserialize)]
    struct TestEvent {
        id: u32,
        message: String,
    }

    // Implement Event trait for TestEvent
    impl Event for TestEvent {
        fn event_type() -> &'static str {
            "test_event"
        }
        
        fn priority() -> EventPriority {
            EventPriority::Normal
        }
        
        fn as_any(&self) -> &dyn Any {
            self
        }
    }

    // Implement StaticEvent to enable fast path
    impl StaticEvent for TestEvent {}

    // Helper to register our TestEvent with the global registry
    fn register_test_event() {
        use crate::events::registry::GlobalTypeRegistry;
        GlobalTypeRegistry::register_static_event_type::<TestEvent>();
        GlobalTypeRegistry::register_with_capacity::<TestEvent>(1000);
        
        // Add a small delay to ensure registration is processed
        std::thread::sleep(std::time::Duration::from_millis(50));
    }

    // Common test for both implementations
    async fn test_publish_receive(event_system: impl EventSystem + 'static) {
        // Register test event first to ensure availability
        register_test_event();
        
        // Start the event system
        event_system.start().await.expect("Failed to start event system");

        // Create a subscriber first
        let mut subscriber = event_system
            .subscribe::<TestEvent>()
            .await
            .expect("Failed to create subscriber");
            
        // Small pause to ensure subscriber is ready
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Create a publisher
        let publisher = event_system.create_publisher::<TestEvent>();

        // Create a test event
        let test_event = TestEvent {
            id: 1,
            message: "Hello, world!".to_string(),
        };

        // Publish the event
        publisher
            .publish(test_event.clone())
            .await
            .expect("Failed to publish event");

        // Receive the event with timeout
        let received = subscriber
            .receive_timeout(Duration::from_secs(1))
            .await
            .expect("Failed to receive event");

        // Verify received event
        assert_eq!(received.id, test_event.id);
        assert_eq!(received.message, test_event.message);

        // Shutdown the event system
        event_system.shutdown().await.expect("Failed to shutdown event system");
    }

    // Common test for batch publishing
    async fn test_batch_publish(event_system: impl EventSystem + 'static) {
        // Register test event first
        register_test_event();
        
        // Start the event system
        event_system.start().await.expect("Failed to start event system");

        // Create a subscriber first
        let mut subscriber = event_system
            .subscribe::<TestEvent>()
            .await
            .expect("Failed to create subscriber");
            
        // Small pause to ensure subscriber is ready
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Create a publisher
        let publisher = event_system.create_publisher::<TestEvent>();

        // Create a batch of test events
        let events: Vec<TestEvent> = (0..5)
            .map(|i| TestEvent {
                id: i,
                message: format!("Batch message {}", i),
            })
            .collect();

        // Publish the batch
        publisher
            .publish_batch(events.clone())
            .await
            .expect("Failed to publish batch");

        // Receive all events
        for _ in 0..5 {
            let received = subscriber
                .receive_timeout(Duration::from_secs(1))
                .await
                .expect("Failed to receive event");

            // Verify received event is one of our batch events
            assert!(events.iter().any(|e| e.id == received.id && e.message == received.message));
        }

        // Shutdown the event system
        event_system.shutdown().await.expect("Failed to shutdown event system");
    }

    // Test try_receive functionality
    async fn test_try_receive(event_system: impl EventSystem + 'static) {
        // Register test event first
        register_test_event();
        
        // Start the event system
        event_system.start().await.expect("Failed to start event system");

        // Create a subscriber first
        let mut subscriber = event_system
            .subscribe::<TestEvent>()
            .await
            .expect("Failed to create subscriber");
            
        // Small pause to ensure subscriber is ready
        tokio::time::sleep(Duration::from_millis(50)).await;

        // Initially, there should be no events
        assert!(subscriber.try_receive().expect("Failed to try_receive").is_none());

        // Create a publisher
        let publisher = event_system.create_publisher::<TestEvent>();

        // Create and publish a test event
        let test_event = TestEvent {
            id: 42,
            message: "Try receive test".to_string(),
        };

        publisher
            .publish(test_event.clone())
            .await
            .expect("Failed to publish event");

        // Small delay to ensure event is processed
        tokio::time::sleep(Duration::from_millis(100)).await;

        // Now try_receive should return Some
        let received = subscriber
            .try_receive()
            .expect("Failed to try_receive")
            .expect("Expected Some, got None");

        // Verify received event
        assert_eq!(received.id, test_event.id);
        assert_eq!(received.message, test_event.message);

        // Shutdown the event system
        event_system.shutdown().await.expect("Failed to shutdown event system");
    }

    // Test static implementation
    #[tokio::test]
    async fn test_static_path() {
        let system = EventSystemBuilder::new()
            .implementation(ImplementationType::StaticFastPath)
            .channel_capacity(1000)
            .build();

        test_publish_receive(system.clone()).await;
        
        let system2 = EventSystemBuilder::new()
            .implementation(ImplementationType::StaticFastPath)
            .channel_capacity(1000)
            .build();
            
        test_batch_publish(system2.clone()).await;

        let system3 = EventSystemBuilder::new()
            .implementation(ImplementationType::StaticFastPath)
            .channel_capacity(1000)
            .build();
            
        test_try_receive(system3).await;
    }

    // Test zero-copy implementation
    #[tokio::test]
    async fn test_zero_copy() {
        let system = EventSystemBuilder::new()
            .implementation(ImplementationType::ZeroCopy)
            .channel_capacity(1000)
            .build();

        test_publish_receive(system.clone()).await;
        
        let system2 = EventSystemBuilder::new()
            .implementation(ImplementationType::ZeroCopy)
            .channel_capacity(1000)
            .build();
            
        test_batch_publish(system2.clone()).await;

        let system3 = EventSystemBuilder::new()
            .implementation(ImplementationType::ZeroCopy)
            .channel_capacity(1000)
            .build();
            
        test_try_receive(system3).await;
    }
} 