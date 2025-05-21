use infra_common::events::api::EventSystem;
use infra_common::events::builder::{EventSystemBuilder, ImplementationType};
use infra_common::events::types::{Event, EventPriority, StaticEvent};
use serde::{Serialize, Deserialize};
use std::sync::Arc;
use std::time::{Duration, Instant};

// Sample event for benchmarking
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

// Static event for high-performance paths
#[derive(Clone, Debug, Serialize, Deserialize)]
struct StaticTestEvent {
    pub id: u64,
    pub data: String,
}

impl Event for StaticTestEvent {
    fn event_type() -> &'static str {
        "static_test_event"
    }

    fn priority() -> EventPriority {
        EventPriority::Normal
    }

    fn as_any(&self) -> &dyn std::any::Any {
        self
    }
}

impl StaticEvent for StaticTestEvent {}

fn create_test_event(id: u64, priority: EventPriority) -> TestEvent {
    TestEvent {
        id,
        data: format!("Event data for event {}", id),
        priority,
    }
}

fn create_static_event(id: u64) -> StaticTestEvent {
    StaticTestEvent {
        id,
        data: format!("Static event data for event {}", id),
    }
}

// API version of single publisher benchmark
async fn bench_api_single_publisher(num_subscribers: usize) {
    println!("Running API Single Publisher benchmark with {} subscribers", num_subscribers);
    
    let start = Instant::now();
    
    // Create Zero Copy system with legacy mode
    let system = EventSystemBuilder::new()
        .implementation(ImplementationType::ZeroCopy)
        .channel_capacity(1024)
        .max_concurrent_dispatches(10000)
        .default_timeout(Some(Duration::from_secs(1)))
        .enable_priority(true)
        .batch_size(100)
        .shard_count(32)
        .build();
    
    // Start the event system
    system.start().await.unwrap();
    
    let counter = Arc::new(std::sync::atomic::AtomicU64::new(0));
    
    // Create subscribers
    let mut subscribers = Vec::new();
    for _ in 0..num_subscribers {
        let subscriber = system.subscribe::<TestEvent>().await.unwrap();
        subscribers.push(subscriber);
    }
    
    // Spawn tasks to count received events
    let tasks: Vec<_> = subscribers.into_iter().map(|mut subscriber| {
        let counter = counter.clone();
        tokio::spawn(async move {
            while let Ok(_event) = subscriber.receive_timeout(Duration::from_secs(5)).await {
                counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        })
    }).collect();
    
    // Create publisher and publish events
    let publisher = system.create_publisher::<TestEvent>();
    
    // Publish events
    for i in 0..100 {
        let event = create_test_event(i, EventPriority::Normal);
        let _ = publisher.publish(event).await;
    }
    
    // Wait for all events to be processed
    let expected_count = num_subscribers as u64 * 100;
    let mut attempts = 0;
    while counter.load(std::sync::atomic::Ordering::Relaxed) < expected_count && attempts < 100 {
        tokio::time::sleep(Duration::from_millis(10)).await;
        attempts += 1;
    }
    
    let elapsed = start.elapsed();
    let count = counter.load(std::sync::atomic::Ordering::Relaxed);
    
    println!("  Time: {:?}, Events: {}, Rate: {:.2} events/sec", 
             elapsed, count, count as f64 / elapsed.as_secs_f64());
    
    // Clean up
    for task in tasks {
        task.abort();
    }
    
    system.shutdown().await.unwrap();
}

// API version of channel benchmark
async fn bench_api_channel(num_subscribers: usize) {
    println!("Running API Channel benchmark with {} subscribers", num_subscribers);
    
    let start = Instant::now();
    
    // Setup with higher concurrency limits
    let system = EventSystemBuilder::new()
        .implementation(ImplementationType::ZeroCopy)
        .channel_capacity(1024)
        .max_concurrent_dispatches(10000)
        .default_timeout(Some(Duration::from_secs(1)))
        .enable_priority(true)
        .batch_size(100)
        .shard_count(32)
        .build();
    
    // Start the event system
    system.start().await.unwrap();
    
    let counter = Arc::new(std::sync::atomic::AtomicU64::new(0));
    
    // Create subscribers
    let mut subscribers = Vec::new();
    for _ in 0..num_subscribers {
        let subscriber = system.subscribe::<TestEvent>().await.unwrap();
        subscribers.push(subscriber);
    }
    
    // Spawn tasks to count received events
    let tasks: Vec<_> = subscribers.into_iter().map(|mut subscriber| {
        let counter = counter.clone();
        tokio::spawn(async move {
            while let Ok(_event) = subscriber.receive_timeout(Duration::from_secs(5)).await {
                counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        })
    }).collect();
    
    // Create publisher and publish events
    let publisher = system.create_publisher::<TestEvent>();
    
    // Publish events
    for i in 1..1001 {
        let event = create_test_event(i, EventPriority::Normal);
        let _ = publisher.publish(event).await;
    }
    
    // Wait for all events to be processed
    let expected_count = num_subscribers as u64 * 1000;
    let mut attempts = 0;
    while counter.load(std::sync::atomic::Ordering::Relaxed) < expected_count && attempts < 100 {
        tokio::time::sleep(Duration::from_millis(10)).await;
        attempts += 1;
    }
    
    let elapsed = start.elapsed();
    let count = counter.load(std::sync::atomic::Ordering::Relaxed);
    
    println!("  Time: {:?}, Events: {}, Rate: {:.2} events/sec", 
             elapsed, count, count as f64 / elapsed.as_secs_f64());
    
    // Clean up
    for task in tasks {
        task.abort();
    }
    
    system.shutdown().await.unwrap();
}

// API version of priority benchmark
async fn bench_api_priority(priority: EventPriority) {
    println!("Running API Priority benchmark with {:?} priority", priority);
    
    let start = Instant::now();
    
    // Setup
    let system = EventSystemBuilder::new()
        .implementation(ImplementationType::ZeroCopy)
        .channel_capacity(1024)
        .max_concurrent_dispatches(10000)
        .default_timeout(Some(Duration::from_secs(1)))
        .enable_priority(true)
        .batch_size(100)
        .shard_count(32)
        .build();
    
    // Start the event system
    system.start().await.unwrap();
    
    let counter = Arc::new(std::sync::atomic::AtomicU64::new(0));
    
    // Create subscribers
    let mut subscribers = Vec::new();
    for _ in 0..10 {
        let subscriber = system.subscribe::<TestEvent>().await.unwrap();
        subscribers.push(subscriber);
    }
    
    // Spawn tasks to count received events
    let tasks: Vec<_> = subscribers.into_iter().map(|mut subscriber| {
        let counter = counter.clone();
        tokio::spawn(async move {
            while let Ok(_event) = subscriber.receive_timeout(Duration::from_secs(5)).await {
                counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        })
    }).collect();
    
    // Create publisher and publish events
    let publisher = system.create_publisher::<TestEvent>();
    
    // Publish events with the specified priority
    for i in 0..100 {
        let event = create_test_event(i, priority);
        let _ = publisher.publish(event).await;
    }
    
    // Wait for all events to be processed
    let expected_count = 10 * 100;
    let mut attempts = 0;
    while counter.load(std::sync::atomic::Ordering::Relaxed) < expected_count && attempts < 100 {
        tokio::time::sleep(Duration::from_millis(10)).await;
        attempts += 1;
    }
    
    let elapsed = start.elapsed();
    let count = counter.load(std::sync::atomic::Ordering::Relaxed);
    
    println!("  Time: {:?}, Events: {}, Rate: {:.2} events/sec", 
             elapsed, count, count as f64 / elapsed.as_secs_f64());
    
    // Clean up
    for task in tasks {
        task.abort();
    }
    
    system.shutdown().await.unwrap();
}

// API version of zero copy benchmark
async fn bench_api_zero_copy(num_subscribers: usize) {
    println!("Running API Zero Copy benchmark with {} subscribers", num_subscribers);
    
    let start = Instant::now();
    
    // Setup event system with high capacity and zero-copy enabled
    let system = EventSystemBuilder::new()
        .implementation(ImplementationType::ZeroCopy)
        .channel_capacity(4096)
        .max_concurrent_dispatches(10000)
        .default_timeout(Some(Duration::from_secs(1)))
        .enable_priority(true)
        .batch_size(100)
        .shard_count(32)
        .build();
    
    // Start the event system
    system.start().await.unwrap();
    
    let counter = Arc::new(std::sync::atomic::AtomicU64::new(0));
    
    // Create subscribers
    let mut subscribers = Vec::new();
    for _ in 0..num_subscribers {
        let subscriber = system.subscribe::<TestEvent>().await.unwrap();
        subscribers.push(subscriber);
    }
    
    // Spawn tasks to count received events
    let tasks: Vec<_> = subscribers.into_iter().map(|mut subscriber| {
        let counter = counter.clone();
        tokio::spawn(async move {
            while let Ok(_event) = subscriber.receive_timeout(Duration::from_secs(5)).await {
                counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        })
    }).collect();
    
    // Create publisher and publish events
    let publisher = system.create_publisher::<TestEvent>();
    
    // Publish events
    for i in 0..1000 {
        let event = create_test_event(i, EventPriority::Normal);
        let _ = publisher.publish(event).await;
    }
    
    // Wait for events to be processed
    let expected_count = num_subscribers as u64 * 1000;
    let mut attempts = 0;
    while counter.load(std::sync::atomic::Ordering::Relaxed) < expected_count && attempts < 100 {
        tokio::time::sleep(Duration::from_millis(10)).await;
        attempts += 1;
    }
    
    let elapsed = start.elapsed();
    let count = counter.load(std::sync::atomic::Ordering::Relaxed);
    
    println!("  Time: {:?}, Events: {}, Rate: {:.2} events/sec", 
             elapsed, count, count as f64 / elapsed.as_secs_f64());
    
    // Clean up
    for task in tasks {
        task.abort();
    }
    
    system.shutdown().await.unwrap();
}

// API version of static fast path benchmark
async fn bench_api_static_fast_path(num_subscribers: usize) {
    println!("Running API Static Fast Path benchmark with {} subscribers", num_subscribers);
    
    // Register the static event type with global registry first
    infra_common::events::registry::GlobalTypeRegistry::register_static_event_type::<StaticTestEvent>();
    infra_common::events::registry::GlobalTypeRegistry::register_with_capacity::<StaticTestEvent>(10000);
    
    let start = Instant::now();
    
    // Create a static fast path event system
    let system = EventSystemBuilder::new()
        .implementation(ImplementationType::StaticFastPath)
        .channel_capacity(10000)
        .build();
    
    // Start the event system
    system.start().await.unwrap();
    
    let counter = Arc::new(std::sync::atomic::AtomicU64::new(0));
    
    // Create subscribers
    let mut subscribers = Vec::new();
    for _ in 0..num_subscribers {
        let subscriber = system.subscribe::<StaticTestEvent>().await.unwrap();
        subscribers.push(subscriber);
    }
    
    // Spawn tasks to count received events
    let tasks: Vec<_> = subscribers.into_iter().map(|mut subscriber| {
        let counter = counter.clone();
        tokio::spawn(async move {
            while let Ok(_event) = subscriber.receive_timeout(Duration::from_secs(5)).await {
                counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        })
    }).collect();
    
    // Create publisher and publish events
    let publisher = system.create_publisher::<StaticTestEvent>();
    
    // Publish a large number of events
    for i in 0..10000 {
        let event = create_static_event(i);
        let _ = publisher.publish(event).await;
    }
    
    // Wait for events to be processed
    let expected_count = num_subscribers as u64 * 10000;
    let mut attempts = 0;
    while counter.load(std::sync::atomic::Ordering::Relaxed) < expected_count && attempts < 100 {
        tokio::time::sleep(Duration::from_millis(10)).await;
        attempts += 1;
    }
    
    let elapsed = start.elapsed();
    let count = counter.load(std::sync::atomic::Ordering::Relaxed);
    
    println!("  Time: {:?}, Events: {}, Rate: {:.2} events/sec", 
             elapsed, count, count as f64 / elapsed.as_secs_f64());
    
    // Clean up
    for task in tasks {
        task.abort();
    }
    
    system.shutdown().await.unwrap();
}

// API version of batch processing benchmark
async fn bench_api_batch_processing(batch_size: usize) {
    println!("Running API Batch Processing benchmark with batch size {}", batch_size);
    
    let start = Instant::now();
    
    // Setup event bus with batch settings
    let system = EventSystemBuilder::new()
        .implementation(ImplementationType::ZeroCopy)
        .channel_capacity(4096)
        .max_concurrent_dispatches(10000)
        .default_timeout(Some(Duration::from_secs(1)))
        .enable_priority(true)
        .batch_size(batch_size)
        .shard_count(32)
        .build();
    
    // Start the event system
    system.start().await.unwrap();
    
    let counter = Arc::new(std::sync::atomic::AtomicU64::new(0));
    
    // Create subscriber
    let mut subscriber = system.subscribe::<TestEvent>().await.unwrap();
    
    // Spawn task to process events
    let counter_clone = counter.clone();
    let task = tokio::spawn(async move {
        while let Ok(_event) = subscriber.receive_timeout(Duration::from_secs(5)).await {
            counter_clone.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    });
    
    // Create publisher
    let publisher = system.create_publisher::<TestEvent>();
    
    // Create batches of events and publish them
    let batches = 10000 / batch_size;
    for j in 0..batches {
        let mut batch = Vec::with_capacity(batch_size);
        for i in 0..batch_size {
            batch.push(create_test_event((j * batch_size + i) as u64, EventPriority::Normal));
        }
        
        // Publish batch
        let _ = publisher.publish_batch(batch).await;
    }
    
    // Wait for events to be processed
    let expected_count = 10000;
    let mut attempts = 0;
    while counter.load(std::sync::atomic::Ordering::Relaxed) < expected_count && attempts < 100 {
        tokio::time::sleep(Duration::from_millis(10)).await;
        attempts += 1;
    }
    
    let elapsed = start.elapsed();
    let count = counter.load(std::sync::atomic::Ordering::Relaxed);
    
    println!("  Time: {:?}, Events: {}, Rate: {:.2} events/sec", 
             elapsed, count, count as f64 / elapsed.as_secs_f64());
    
    // Clean up
    task.abort();
    system.shutdown().await.unwrap();
}

// Run the API benchmarks
#[tokio::main]
async fn main() {
    println!("\n----- API Benchmarks Using Public Interface -----\n");
    
    // Run single publisher benchmarks
    for &subscribers in &[1, 10, 100] {
        bench_api_single_publisher(subscribers).await;
    }
    
    println!();
    
    // Run channel benchmarks
    for &subscribers in &[10, 100, 1000] {
        bench_api_channel(subscribers).await;
    }
    
    println!();
    
    // Run priority benchmarks
    for &priority in &[EventPriority::Low, EventPriority::Normal, EventPriority::High, EventPriority::Critical] {
        bench_api_priority(priority).await;
    }
    
    println!();
    
    // Run zero copy benchmarks
    for &subscribers in &[1, 10, 100, 1000] {
        bench_api_zero_copy(subscribers).await;
    }
    
    println!();
    
    // Run static fast path benchmarks
    for &subscribers in &[10, 100, 1000] {
        bench_api_static_fast_path(subscribers).await;
    }
    
    println!();
    
    // Run batch processing benchmarks
    for &batch_size in &[10, 100, 1000] {
        bench_api_batch_processing(batch_size).await;
    }
} 