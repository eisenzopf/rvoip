# infra-common: High-Performance Event System for RVOIP

This crate provides a highly efficient event system with multiple implementation options including a zero-copy architecture that can handle millions of events per second with minimal latency.

## Key Features

- **Dual Implementation**: Choose between Zero Copy or Static Fast Path based on your needs
- **Zero-Copy Architecture**: Uses `Arc<T>` for events to eliminate serialization/deserialization overhead
- **Event Filtering**: Filter events based on content before they reach subscribers
- **Batch Processing**: Support for batched event processing to increase throughput
- **Priority System**: Event prioritization with critical events handled first
- **Unified API**: Consistent API across different implementation types
- **Timeout Controls**: Fine-grained control over receiving timeouts

## Performance

The event system has been benchmarked to handle:

- Up to 2.2 million events/second with the Zero Copy implementation (5 subscribers)
- Up to 900,000 events/second with the Static Fast Path implementation (5 subscribers)
- Sub-millisecond latency for critical events

## RVOIP Architecture Integration

For achieving high performance in the RVOIP stack supporting up to 100,000 concurrent calls, this event system should be integrated using a hybrid approach across all libraries:

### Standardized Implementation Approach

All RVOIP libraries should implement a **Zero Copy + Priority Handling** model:

1. **Use Zero Copy for Standard Messages**:
   - Use the Zero Copy implementation for most protocol messages (SIP, RTP, media commands)
   - Leverage the high throughput (2M+ events/sec)
   - Zero serialization overhead, minimal memory allocation

2. **Use Priority System for State Changes**:
   - `EventPriority::Critical`: Call setup/teardown and critical operations
   - `EventPriority::High`: Mid-call state changes and important media events
   - `EventPriority::Normal`: Regular messaging and protocol flow
   - `EventPriority::Low`: Metrics, logging, and background operations

3. **Event Filtering for Targeted Processing**:
   - Use event filtering to route events to specific subscribers based on content
   - Reduce processing overhead by filtering early

4. **Batch Processing for High-Volume Events**:
   - Use batch processing for RTP packets, media samples, and similar high-volume data
   - Ideal for metrics collection and non-critical events

### Library-Specific Event Strategy

Each RVOIP library should follow these implementation guidelines:

- **transaction-core**: Use Zero Copy for SIP transactions, critical priority for transaction state changes
- **sip-transport**: Use Zero Copy for network events, prioritize connection issues
- **session-core**: Use Critical priority for call control, High priority for signaling
- **rtp-core**: Use Zero Copy for packets, batch processing for statistics
- **media-core**: Use High priority for media controls, batch processing for media frames

## Usage Examples

### Creating an Event System

The unified `EventSystem` API provides a consistent interface for working with both implementation types:

```rust
use infra_common::events::builder::{EventSystemBuilder, ImplementationType};
use infra_common::events::types::{Event, EventPriority};
use infra_common::events::api::EventSystem;
use std::any::Any;
use std::time::Duration;
use std::sync::Arc;
use serde::{Serialize, Deserialize};

// Define your event
#[derive(Clone, Debug, Serialize, Deserialize)]
struct MyEvent {
    id: u32,
    message: String,
}

// Implement Event trait for the event
impl Event for MyEvent {
    fn event_type() -> &'static str {
        "my_event"
    }
    
    fn priority() -> EventPriority {
        EventPriority::Normal
    }
    
    fn as_any(&self) -> &dyn Any {
        self
    }
}

// Create an event system with the builder
let system = EventSystemBuilder::new()
    .implementation(ImplementationType::ZeroCopy)  // or StaticFastPath
    .channel_capacity(1000)
    .max_concurrent_dispatches(500)
    .enable_priority(true)
    .build();

// Start the event system
system.start().await.expect("Failed to start event system");
```

### Basic Publishing and Subscribing

```rust
// Continue from previous example...

// Create a subscriber
let mut subscriber = system.subscribe::<MyEvent>()
    .await
    .expect("Failed to create subscriber");

// Create a publisher
let publisher = system.create_publisher::<MyEvent>();

// Create and publish an event
let event = MyEvent {
    id: 1,
    message: "Hello, World!".to_string(),
};
publisher.publish(event).await.expect("Failed to publish event");

// Receive the event
let received = subscriber
    .receive_timeout(Duration::from_secs(1))
    .await
    .expect("Failed to receive event");

println!("Received: id={}, message={}", received.id, received.message);
```

### Event Filtering

Filter events based on their content to process only relevant events:

```rust
use infra_common::events::api::filters;

// Method 1: Subscribe with a filter function
let mut filtered_subscriber = system.subscribe_filtered::<MyEvent, _>(|event| {
    event.id > 100 && event.message.contains("important")
}).await.expect("Failed to create filtered subscriber");

// Method 2: Using predefined filter utilities
// Filter for events with a specific field value
let id_filter = filters::field_equals(|e: &MyEvent| &e.id, 42);

// Filter for events where a field matches a predicate
let message_filter = filters::field_matches(|e: &MyEvent| &e.message, |msg| msg.starts_with("SIP"));

// Combine filters with logical operations
let combined_filter = filters::and(id_filter, message_filter);

// Subscribe with the combined filter
let mut complex_subscriber = system.subscribe_with_filter::<MyEvent>(combined_filter)
    .await
    .expect("Failed to create subscriber with complex filter");

// Publish events - only matching ones will be received by the filtered subscribers
for i in 0..200 {
    publisher.publish(MyEvent {
        id: i,
        message: if i % 5 == 0 { "SIP: important message".to_string() } else { "Regular message".to_string() },
    }).await.expect("Failed to publish");
}

// The filtered_subscriber will only receive events with id > 100 and contains "important"
// The complex_subscriber will only receive events with id == 42 and message starting with "SIP"
```

### Batch Processing

For higher throughput, use batch processing:

```rust
// Create a batch of events
let events: Vec<MyEvent> = (0..100)
    .map(|i| MyEvent {
        id: i,
        message: format!("Batch message {}", i),
    })
    .collect();

// Publish the batch
publisher.publish_batch(events).await.expect("Failed to publish batch");

// Receive all events
for _ in 0..100 {
    let received = subscriber
        .receive_timeout(Duration::from_secs(1))
        .await
        .expect("Failed to receive event");
    
    println!("Received batch event: {}", received.id);
}
```

### Non-Blocking Receive

Use `try_receive` for non-blocking operations:

```rust
// Check if there are any events available without blocking
match subscriber.try_receive() {
    Ok(Some(event)) => println!("Got event without blocking: {}", event.id),
    Ok(None) => println!("No events available right now"),
    Err(e) => eprintln!("Error trying to receive: {}", e),
}
```

### Shutdown

Always shut down the event system when done:

```rust
// Shutdown the event system
system.shutdown().await.expect("Failed to shutdown event system");
```

## Implementation Details

### Zero Copy Implementation

The Zero Copy implementation uses tokio broadcast channels wrapped in Arc references, providing:

- High throughput (2M+ events/second with 5 subscribers)
- Thread-safe sharing of events without cloning the event data
- Automatic cleanup when all references are dropped

```rust
// Example of creating a Zero Copy event system
let system = EventSystemBuilder::new()
    .implementation(ImplementationType::ZeroCopy)
    .channel_capacity(10_000)        // Buffer size for channels
    .max_concurrent_dispatches(1000)  // Max concurrent event dispatching
    .enable_priority(true)           // Enable prioritized event handling
    .default_timeout(Some(Duration::from_secs(1)))  // Default timeout
    .shard_count(8)                  // Number of shards for performance
    .build();
```

### Static Fast Path Implementation

For specialized scenarios where type safety at compile time is preferred:

```rust
use infra_common::events::types::StaticEvent;

// Implement StaticEvent for fast path eligibility 
impl StaticEvent for MyEvent {}

// Create a Static Fast Path event system
let system = EventSystemBuilder::new()
    .implementation(ImplementationType::StaticFastPath)
    .channel_capacity(1000)
    .build();
```

## Performance Considerations

- The Zero Copy implementation provides the highest throughput in most scenarios
- Performance scales inversely with the number of subscribers (due to broadcast channel mechanics)
- Batch processing significantly increases throughput for high-volume events
- Filtering events early reduces processing overhead and improves overall system performance
- For specialized high-performance needs, implement custom filtering at the subscriber level 