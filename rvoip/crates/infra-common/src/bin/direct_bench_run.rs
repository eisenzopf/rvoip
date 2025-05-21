use infra_common::events::bus::{EventBus, EventBusConfig, Publisher};
use infra_common::events::types::{Event, EventPriority, StaticEvent};
use infra_common::events::registry::GlobalTypeRegistry;
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

// Direct version of single publisher benchmark
async fn bench_direct_single_publisher(num_subscribers: usize) {
    println!("Running DIRECT Single Publisher benchmark with {} subscribers", num_subscribers);
    
    let start = Instant::now();
    
    // Setup event bus with high capacity
    let event_bus = EventBus::with_config(EventBusConfig {
        max_concurrent_dispatches: 10000,
        default_timeout: Duration::from_secs(1),
        shard_count: 32,
        broadcast_capacity: 1024,
        enable_priority: true,
        enable_zero_copy: false, // Use legacy mode
        batch_size: 100,
    });
    
    let counter = Arc::new(std::sync::atomic::AtomicU64::new(0));
    
    // Create handler function
    struct TestHandler {
        counter: Arc<std::sync::atomic::AtomicU64>,
    }
    
    #[async_trait::async_trait]
    impl infra_common::events::types::EventHandler<TestEvent> for TestHandler {
        async fn handle(&self, _event: TestEvent) {
            self.counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }
    
    // Register subscribers
    for _ in 0..num_subscribers {
        let handler = TestHandler { counter: counter.clone() };
        let _ = event_bus.subscribe::<TestEvent, _>(None, handler).await.unwrap();
    }
    
    // Publish events using direct event bus
    for i in 0..100 {
        let _ = event_bus.publish(create_test_event(i, EventPriority::Normal)).await;
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
}

// Direct version of channel benchmark
async fn bench_direct_channel(num_subscribers: usize) {
    println!("Running DIRECT Channel benchmark with {} subscribers", num_subscribers);
    
    let start = Instant::now();
    
    // Setup with higher concurrency limits
    let event_bus = EventBus::with_config(EventBusConfig {
        max_concurrent_dispatches: 10000,
        default_timeout: Duration::from_secs(1),
        shard_count: 32,
        broadcast_capacity: 1024,
        enable_priority: true,
        enable_zero_copy: true,
        batch_size: 100,
    });
    
    let counter = Arc::new(std::sync::atomic::AtomicU64::new(0));
    
    // Pre-register a dummy subscriber to ensure the broadcast channel is created
    struct TestHandler {
        counter: Arc<std::sync::atomic::AtomicU64>,
    }
    
    #[async_trait::async_trait]
    impl infra_common::events::types::EventHandler<TestEvent> for TestHandler {
        async fn handle(&self, _event: TestEvent) {
            self.counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }
    
    let dummy_handler = TestHandler { counter: counter.clone() };
    let _ = event_bus.subscribe::<TestEvent, _>(None, dummy_handler).await.unwrap();
    
    // Publish one event to ensure the broadcast channel is created
    let _ = event_bus.publish(create_test_event(0, EventPriority::Normal)).await;
    
    // Create channel subscribers
    let mut receivers = Vec::new();
    for _ in 0..num_subscribers {
        let rx = event_bus.subscribe_broadcast::<TestEvent>().await.unwrap();
        receivers.push(rx);
    }
    
    // Setup counters for each receiver
    let counter_tasks: Vec<_> = receivers.into_iter().map(|mut rx| {
        let counter = counter.clone();
        tokio::spawn(async move {
            while let Ok(_event) = rx.recv().await {
                counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        })
    }).collect();
    
    // Publish events
    for i in 1..1001 {
        let _ = event_bus.publish(create_test_event(i, EventPriority::Normal)).await;
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
    
    // Explicitly drop the counter tasks to avoid task leaks
    for task in counter_tasks {
        task.abort();
    }
}

// Direct version of priority benchmark
async fn bench_direct_priority(priority: EventPriority) {
    println!("Running DIRECT Priority benchmark with {:?} priority", priority);
    
    let start = Instant::now();
    
    // Setup
    let event_bus = EventBus::with_config(EventBusConfig {
        max_concurrent_dispatches: 10000,
        default_timeout: Duration::from_secs(1),
        shard_count: 32,
        broadcast_capacity: 1024,
        enable_priority: true,
        enable_zero_copy: true,
        batch_size: 100,
    });
    
    let counter = Arc::new(std::sync::atomic::AtomicU64::new(0));
    
    // Create handler function
    struct TestHandler {
        counter: Arc<std::sync::atomic::AtomicU64>,
    }
    
    #[async_trait::async_trait]
    impl infra_common::events::types::EventHandler<TestEvent> for TestHandler {
        async fn handle(&self, _event: TestEvent) {
            self.counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    }
    
    // Register subscribers with different priorities
    for _ in 0..10 {
        let handler = TestHandler { counter: counter.clone() };
        let _ = event_bus.subscribe::<TestEvent, _>(None, handler).await.unwrap();
    }
    
    // Publish events with the specified priority
    for i in 0..100 {
        let _ = event_bus.publish(create_test_event(i, priority)).await;
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
}

// Direct version of zero copy benchmark
async fn bench_direct_zero_copy(num_subscribers: usize) {
    println!("Running DIRECT Zero Copy benchmark with {} subscribers", num_subscribers);
    
    let start = Instant::now();
    
    // Setup event bus with high capacity and zero-copy enabled
    let event_bus = EventBus::with_config(EventBusConfig {
        max_concurrent_dispatches: 10000,
        default_timeout: Duration::from_secs(1),
        shard_count: 32,
        broadcast_capacity: 4096,
        enable_priority: true,
        enable_zero_copy: true,
        batch_size: 100,
    });
    
    let counter = Arc::new(std::sync::atomic::AtomicU64::new(0));
    
    // Create subscribers via broadcast channels
    let mut receivers = Vec::new();
    for _ in 0..num_subscribers {
        let rx = event_bus.subscribe_broadcast::<TestEvent>().await.unwrap();
        receivers.push(rx);
    }
    
    // Spawn tasks to count received events
    let tasks: Vec<_> = receivers.into_iter().map(|mut rx| {
        let counter = counter.clone();
        tokio::spawn(async move {
            while let Ok(_event) = rx.recv().await {
                counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        })
    }).collect();
    
    // Publish events
    for i in 0..1000 {
        let _ = event_bus.publish(create_test_event(i, EventPriority::Normal)).await;
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
    
    for task in tasks {
        task.abort();
    }
}

// Direct version of static fast path benchmark
async fn bench_direct_static_fast_path(num_subscribers: usize) {
    println!("Running DIRECT Static Fast Path benchmark with {} subscribers", num_subscribers);
    
    // Register the static event type with the registry
    GlobalTypeRegistry::register_static_event_type::<StaticTestEvent>();
    GlobalTypeRegistry::register_with_capacity::<StaticTestEvent>(10000);
    
    let start = Instant::now();
    
    let counter = Arc::new(std::sync::atomic::AtomicU64::new(0));
    
    // Create subscribers via global registry
    let tasks: Vec<_> = (0..num_subscribers).map(|_| {
        let counter = counter.clone();
        let mut rx = GlobalTypeRegistry::subscribe::<StaticTestEvent>();
        tokio::spawn(async move {
            while let Ok(_event) = rx.recv().await {
                counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        })
    }).collect();
    
    // Get sender from global registry
    let sender = GlobalTypeRegistry::get_sender::<StaticTestEvent>();
    
    // Publish a large number of events
    for i in 0..10000 {
        let event = create_static_event(i);
        let _ = sender.send(Arc::new(event));
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
    
    for task in tasks {
        task.abort();
    }
}

// Direct version of batch processing benchmark
async fn bench_direct_batch_processing(batch_size: usize) {
    println!("Running DIRECT Batch Processing benchmark with batch size {}", batch_size);
    
    let start = Instant::now();
    
    // Setup event bus with batch settings
    let event_bus = EventBus::with_config(EventBusConfig {
        max_concurrent_dispatches: 10000,
        default_timeout: Duration::from_secs(1),
        shard_count: 32,
        broadcast_capacity: 4096,
        enable_priority: true,
        enable_zero_copy: true,
        batch_size: batch_size,
    });
    
    let counter = Arc::new(std::sync::atomic::AtomicU64::new(0));
    let publisher = Publisher::<TestEvent>::new(event_bus.clone());
    
    // Subscribe to events
    let mut rx = event_bus.subscribe_broadcast::<TestEvent>().await.unwrap();
    let counter_clone = counter.clone();
    let task = tokio::spawn(async move {
        while let Ok(_event) = rx.recv().await {
            counter_clone.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
        }
    });
    
    // Create batch of events
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
    
    task.abort();
}

// Run the direct benchmarks
#[tokio::main]
async fn main() {
    println!("\n----- DIRECT Benchmarks Using Low-Level Implementation -----\n");
    
    // Run single publisher benchmarks
    for &subscribers in &[1, 10, 100] {
        bench_direct_single_publisher(subscribers).await;
    }
    
    println!();
    
    // Run channel benchmarks
    for &subscribers in &[10, 100, 1000] {
        bench_direct_channel(subscribers).await;
    }
    
    println!();
    
    // Run priority benchmarks
    for &priority in &[EventPriority::Low, EventPriority::Normal, EventPriority::High, EventPriority::Critical] {
        bench_direct_priority(priority).await;
    }
    
    println!();
    
    // Run zero copy benchmarks
    for &subscribers in &[1, 10, 100, 1000] {
        bench_direct_zero_copy(subscribers).await;
    }
    
    println!();
    
    // Run static fast path benchmarks
    for &subscribers in &[10, 100, 1000] {
        bench_direct_static_fast_path(subscribers).await;
    }
    
    println!();
    
    // Run batch processing benchmarks
    for &batch_size in &[10, 100, 1000] {
        bench_direct_batch_processing(batch_size).await;
    }
} 