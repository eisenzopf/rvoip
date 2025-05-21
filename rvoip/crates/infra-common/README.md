# infra-common: High-Performance Event Bus for RVOIP

This crate provides a zero-copy, lock-free event bus architecture that can handle 250,000+ events per second with sub-millisecond latency.

## Key Features

- **Zero-Copy Architecture**: Uses `Arc<T>` for events to eliminate serialization/deserialization overhead
- **Lock-Free Data Structures**: Employs `DashMap` for concurrent access without contention
- **Fast Static Paths**: Optimized code paths for high-frequency events
- **Batch Processing**: Support for batched event processing to increase throughput
- **Priority System**: Event prioritization with critical events handled first
- **Memory Optimization**: Uses mimalloc for efficient memory allocation

## Performance

The event bus has been benchmarked to handle:

- Up to 700,000+ events/second with static event paths
- 190,000+ events/second with standard channels
- Sub-millisecond latency for critical events

## RVOIP Architecture Integration

For achieving high performance in the RVOIP stack supporting up to 100,000 concurrent calls, this event bus should be integrated using a hybrid approach across all libraries:

### Standardized Implementation Approach

All RVOIP libraries should implement a **StaticEvent Fast Path + Priority Handling** model:

1. **Use StaticEvent for Protocol Messages**:
   - Implement `StaticEvent` for all standard protocol messages (SIP, RTP, media commands)
   - These use the optimized fast path (700K+ events/sec throughput)
   - Zero serialization overhead, minimal memory allocation

2. **Use Priority System for State Changes**:
   - `EventPriority::Critical`: Call setup/teardown and critical operations
   - `EventPriority::High`: Mid-call state changes and important media events
   - `EventPriority::Normal`: Regular messaging and protocol flow
   - `EventPriority::Low`: Metrics, logging, and background operations

3. **Batch Processing for High-Volume Events**:
   - Use batch processing for RTP packets, media samples, and similar high-volume data
   - Ideal for metrics collection and non-critical events

### Library-Specific Event Strategy

Each RVOIP library should follow these implementation guidelines:

- **transaction-core**: Use StaticEvent for SIP transactions, critical priority for transaction state changes
- **sip-transport**: Use StaticEvent for network events, prioritize connection issues
- **session-core**: Use Critical priority for call control, High priority for signaling
- **rtp-core**: Use StaticEvent for packets, batch processing for statistics
- **media-core**: Use High priority for media controls, batch processing for media frames

## Usage Examples

### Unified Event System API

The unified `EventSystem` API provides a consistent interface for working with both implementation types:

```rust
use infra_common::events::system::{EventSystem, EventPublisher, EventSubscriber};
use infra_common::events::builder::{EventSystemBuilder, ImplementationType};
use infra_common::events::types::{Event, StaticEvent};
use std::time::Duration;

// Define your event
#[derive(Clone, Debug, Serialize, Deserialize)]
struct MyEvent {
    id: u64,
    data: String,
}

impl Event for MyEvent {
    fn event_type() -> &'static str {
        "my_event"
    }
}

// Implement StaticEvent for fast path eligibility 
impl StaticEvent for MyEvent {}

// Create an event system with the builder
let event_system = EventSystemBuilder::new()
    .implementation(ImplementationType::StaticFastPath)  // or ZeroCopy
    .channel_capacity(10_000)
    .build();

// Start the event system
event_system.start().await.unwrap();

// Create a publisher
let publisher = event_system.create_publisher::<MyEvent>();

// Subscribe to events
let mut subscriber = event_system.subscribe::<MyEvent>().await.unwrap();

// Publish an event
publisher.publish(MyEvent { 
    id: 1, 
    data: "Hello".to_string() 
}).await.unwrap();

// Receive the event
if let Ok(event) = subscriber.receive().await {
    println!("Received event with id: {}", event.id);
}

// Publish a batch of events for higher throughput
let mut batch = Vec::with_capacity(100);
for i in 0..100 {
    batch.push(MyEvent { 
        id: i, 
        data: format!("Event {}", i) 
    });
}
publisher.publish_batch(batch).await.unwrap();

// Receive with timeout
if let Ok(event) = subscriber.receive_timeout(Duration::from_millis(100)).await {
    println!("Received event: {}", event.id);
}

// Check if messages are available without blocking
if let Ok(Some(event)) = subscriber.try_receive() {
    println!("Got event without blocking: {}", event.id);
}

// Shutdown when done
event_system.shutdown().await.unwrap();
```

### Basic Publishing and Subscribing

```rust
use infra_common::events::bus::EventBus;
use infra_common::events::types::{Event, EventHandler};
use async_trait::async_trait;

// Define your event
#[derive(Clone, Debug, Serialize, Deserialize)]
struct MyEvent {
    id: u64,
    data: String,
}

impl Event for MyEvent {
    fn event_type() -> &'static str {
        "my_event"
    }
}

// Define a handler
struct MyHandler;

#[async_trait]
impl EventHandler<MyEvent> for MyHandler {
    async fn handle(&self, event: MyEvent) {
        println!("Received event: {:?}", event);
    }
}

// In your application
async fn setup() {
    let event_bus = EventBus::new();
    
    // Subscribe to events
    let handler = MyHandler;
    event_bus.subscribe::<MyEvent, _>(None, handler).await.unwrap();
    
    // Publish an event
    let event = MyEvent { id: 1, data: "Hello".to_string() };
    event_bus.publish(event).await.unwrap();
}
```

### High-Performance Static Events

For maximum performance, implement the `StaticEvent` trait:

```rust
use infra_common::events::types::{Event, StaticEvent};

#[derive(Clone, Debug, Serialize, Deserialize)]
struct MyStaticEvent {
    id: u64,
    data: String,
}

impl Event for MyStaticEvent {
    fn event_type() -> &'static str {
        "my_static_event"
    }
}

// Implement StaticEvent for fast path processing
impl StaticEvent for MyStaticEvent {}

// Use FastPublisher for maximum throughput
let publisher = FastPublisher::<MyStaticEvent>::new();
publisher.publish(event).await.unwrap();
```

### Batch Processing

```rust
// Create a batch of events
let mut batch = Vec::with_capacity(100);
for i in 0..100 {
    batch.push(MyEvent { id: i, data: format!("Event {}", i) });
}

// Publish batch for higher throughput
let publisher = Publisher::<MyEvent>::new(event_bus);
publisher.publish_batch(batch).await.unwrap();
```

## Configuration

The event bus can be configured for different performance characteristics:

```rust
let event_bus = EventBus::with_config(EventBusConfig {
    max_concurrent_dispatches: 10000,
    default_timeout: Duration::from_secs(1),
    broadcast_capacity: 16384,
    enable_priority: true,
    enable_zero_copy: true,
    batch_size: 100,
    shard_count: 32,
});
``` 