use infra_common::events::api::EventSystem;
use infra_common::events::builder::{EventSystemBuilder, ImplementationType};
use infra_common::events::registry::GlobalTypeRegistry;
use infra_common::events::types::{Event, EventPriority, StaticEvent};
use serde::{Serialize, Deserialize};
use std::any::Any;
use std::time::Duration;

/// Simple example that demonstrates how to use the Static Fast Path implementation
/// of the event system through the public API.

// Define a simple event type
#[derive(Clone, Debug, Serialize, Deserialize)]
struct SimpleEvent {
    id: u32,
    message: String,
}

// Implement the Event trait
impl Event for SimpleEvent {
    fn event_type() -> &'static str {
        "simple_event"
    }
    
    fn priority() -> EventPriority {
        EventPriority::Normal
    }
    
    fn as_any(&self) -> &dyn Any {
        self
    }
}

// Implement StaticEvent for fast path processing
impl StaticEvent for SimpleEvent {}

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Static Fast Path Example");
    println!("=======================");
    
    // Register our event type with the global registry (important for static fast path)
    GlobalTypeRegistry::register_static_event_type::<SimpleEvent>();
    GlobalTypeRegistry::register_with_capacity::<SimpleEvent>(1000);
    
    // Create a static fast path event system using the builder
    let system = EventSystemBuilder::new()
        .implementation(ImplementationType::StaticFastPath)
        .channel_capacity(1000)
        .build();
    
    // Start the event system
    system.start().await?;
    println!("Event system started");
    
    // Create a subscriber first (important for static path implementation)
    println!("Creating subscriber...");
    let mut subscriber = system.subscribe::<SimpleEvent>().await?;
    
    // Small delay to ensure subscriber is ready
    tokio::time::sleep(Duration::from_millis(50)).await;
    
    // Create a publisher
    println!("Creating publisher...");
    let publisher = system.create_publisher::<SimpleEvent>();
    
    // Publish some events
    println!("Publishing events...");
    for i in 0..5 {
        let event = SimpleEvent {
            id: i,
            message: format!("Hello from Static Fast Path, message #{}", i),
        };
        
        println!("Publishing: {{ id: {}, message: \"{}\" }}", event.id, event.message);
        publisher.publish(event).await?;
        
        // Small delay between publishing events
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    
    // Receive events
    println!("\nReceiving events...");
    for _ in 0..5 {
        match subscriber.receive_timeout(Duration::from_secs(1)).await {
            Ok(event) => println!("Received: {{ id: {}, message: \"{}\" }}", event.id, event.message),
            Err(e) => println!("Error receiving event: {}", e),
        }
    }
    
    // Shutdown the event system
    println!("\nShutting down event system...");
    system.shutdown().await?;
    println!("Event system shut down successfully");
    
    Ok(())
} 