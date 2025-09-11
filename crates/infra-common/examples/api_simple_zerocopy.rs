use rvoip_infra_common::events::api::EventSystem;
use rvoip_infra_common::events::builder::{EventSystemBuilder, ImplementationType};
use rvoip_infra_common::events::types::{Event, EventPriority};
use serde::{Serialize, Deserialize};
use std::any::Any;
use std::time::Duration;

/// Simple example that demonstrates how to use the Zero Copy implementation
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Zero Copy Event System Example");
    println!("=============================");
    
    // Create a zero-copy event system using the builder
    let system = EventSystemBuilder::new()
        .implementation(ImplementationType::ZeroCopy)
        .channel_capacity(1000)
        .max_concurrent_dispatches(500)
        .enable_priority(true)
        .default_timeout(Some(Duration::from_secs(1)))
        .batch_size(100)
        .shard_count(8)
        .build();
    
    // Start the event system
    system.start().await?;
    println!("Event system started");
    
    // Create a subscriber
    println!("Creating subscriber...");
    let mut subscriber = system.subscribe::<SimpleEvent>().await?;
    
    // Create a publisher
    println!("Creating publisher...");
    let publisher = system.create_publisher::<SimpleEvent>();
    
    // Publish some events
    println!("Publishing events...");
    for i in 0..5 {
        let event = SimpleEvent {
            id: i,
            message: format!("Hello from Zero Copy, message #{}", i),
        };
        
        println!("Publishing: {{ id: {}, message: \"{}\" }}", event.id, event.message);
        publisher.publish(event).await?;
        
        // Small delay between publishing events
        tokio::time::sleep(Duration::from_millis(10)).await;
    }
    
    // Batch publishing example
    println!("\nBatch publishing example...");
    let batch: Vec<SimpleEvent> = (5..10)
        .map(|i| SimpleEvent {
            id: i,
            message: format!("Batch message #{}", i),
        })
        .collect();
    
    println!("Publishing batch of {} events...", batch.len());
    publisher.publish_batch(batch).await?;
    
    // Receive events (both individual and batch published)
    println!("\nReceiving events...");
    for _ in 0..10 {
        match subscriber.receive_timeout(Duration::from_secs(1)).await {
            Ok(event) => println!("Received: {{ id: {}, message: \"{}\" }}", event.id, event.message),
            Err(e) => println!("Error receiving event: {}", e),
        }
    }
    
    // Try non-blocking receive
    println!("\nDemonstrating try_receive (non-blocking)...");
    match subscriber.try_receive() {
        Ok(Some(event)) => println!("Received: {{ id: {}, message: \"{}\" }}", event.id, event.message),
        Ok(None) => println!("No events available for immediate consumption"),
        Err(e) => println!("Error with try_receive: {}", e),
    }
    
    // Shutdown the event system
    println!("\nShutting down event system...");
    system.shutdown().await?;
    println!("Event system shut down successfully");
    
    Ok(())
} 