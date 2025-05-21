use infra_common::events::builder::{EventSystemBuilder, ImplementationType};
use infra_common::events::types::{Event, EventPriority, StaticEvent};
use infra_common::events::api::EventSystem;
use infra_common::events::registry::GlobalTypeRegistry;
use infra_common::events::bus::{EventBus, EventBusConfig, Publisher};
use serde::{Serialize, Deserialize};
use std::any::Any;
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

// Benchmark the Public API Zero Copy implementation
async fn bench_api_zero_copy(num_subscribers: usize, num_events: usize) -> (Duration, u64) {
    println!("Running Zero Copy API benchmark with {} subscribers, {} events", 
             num_subscribers, num_events);
    
    let start = Instant::now();
    
    // Setup event system with high capacity and zero-copy enabled
    let system = EventSystemBuilder::new()
        .implementation(ImplementationType::ZeroCopy)
        .channel_capacity(4096)
        .max_concurrent_dispatches(10000)
        .enable_priority(true)
        .default_timeout(Some(Duration::from_secs(1)))
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
    
    // Create publisher
    let publisher = system.create_publisher::<TestEvent>();
    
    // Publish events
    for i in 0..num_events {
        let event = create_test_event(i as u64, EventPriority::Normal);
        let _ = publisher.publish(event).await;
    }
    
    // Wait for events to be processed
    let expected_count = num_subscribers as u64 * num_events as u64;
    let mut attempts = 0;
    while counter.load(std::sync::atomic::Ordering::Relaxed) < expected_count && attempts < 100 {
        tokio::time::sleep(Duration::from_millis(10)).await;
        attempts += 1;
    }
    
    let elapsed = start.elapsed();
    
    // Clean up
    for task in tasks {
        task.abort();
    }
    
    system.shutdown().await.unwrap();
    
    (elapsed, counter.load(std::sync::atomic::Ordering::Relaxed))
}

// Benchmark the Public API Static Fast Path implementation
async fn bench_api_static_fast_path(num_subscribers: usize, num_events: usize) -> (Duration, u64) {
    println!("Running Static Fast Path API benchmark with {} subscribers, {} events", 
             num_subscribers, num_events);
    
    // Register the static event type
    GlobalTypeRegistry::register_static_event_type::<StaticTestEvent>();
    GlobalTypeRegistry::register_with_capacity::<StaticTestEvent>(10000);
    
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
    
    // Publish events
    for i in 0..num_events {
        let event = create_static_event(i as u64);
        let _ = publisher.publish(event).await;
    }
    
    // Wait for events to be processed
    let expected_count = num_subscribers as u64 * num_events as u64;
    let mut attempts = 0;
    while counter.load(std::sync::atomic::Ordering::Relaxed) < expected_count && attempts < 100 {
        tokio::time::sleep(Duration::from_millis(10)).await;
        attempts += 1;
    }
    
    let elapsed = start.elapsed();
    
    // Clean up
    for task in tasks {
        task.abort();
    }
    
    system.shutdown().await.unwrap();
    
    (elapsed, counter.load(std::sync::atomic::Ordering::Relaxed))
}

// Direct implementation benchmark for Zero Copy with no API layer
async fn bench_direct_zero_copy(num_subscribers: usize, num_events: usize) -> (Duration, u64) {
    println!("Running DIRECT Zero Copy benchmark with {} subscribers, {} events", 
             num_subscribers, num_events);
    
    let start = Instant::now();
    
    // Setup event bus directly
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
    
    // Create channel subscribers directly
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
    
    // Create publisher directly
    let publisher = Publisher::<TestEvent>::new(event_bus.clone());
    
    // Publish events
    for i in 0..num_events {
        let event = create_test_event(i as u64, EventPriority::Normal);
        let _ = publisher.publish(event).await;
    }
    
    // Wait for events to be processed
    let expected_count = num_subscribers as u64 * num_events as u64;
    let mut attempts = 0;
    while counter.load(std::sync::atomic::Ordering::Relaxed) < expected_count && attempts < 100 {
        tokio::time::sleep(Duration::from_millis(10)).await;
        attempts += 1;
    }
    
    let elapsed = start.elapsed();
    
    // Clean up
    for task in tasks {
        task.abort();
    }
    
    (elapsed, counter.load(std::sync::atomic::Ordering::Relaxed))
}

// Direct implementation benchmark for Static Fast Path with no API layer
async fn bench_direct_static_fast_path(num_subscribers: usize, num_events: usize) -> (Duration, u64) {
    println!("Running DIRECT Static Fast Path benchmark with {} subscribers, {} events", 
             num_subscribers, num_events);
    
    // Register the static event type
    GlobalTypeRegistry::register_static_event_type::<StaticTestEvent>();
    GlobalTypeRegistry::register_with_capacity::<StaticTestEvent>(10000);
    
    let start = Instant::now();
    
    let counter = Arc::new(std::sync::atomic::AtomicU64::new(0));
    
    // Create direct receivers
    let tasks: Vec<_> = (0..num_subscribers).map(|_| {
        let counter = counter.clone();
        let mut rx = GlobalTypeRegistry::subscribe::<StaticTestEvent>();
        tokio::spawn(async move {
            while let Ok(_event) = rx.recv().await {
                counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
            }
        })
    }).collect();
    
    // Create a direct sender
    let sender = GlobalTypeRegistry::get_sender::<StaticTestEvent>();
    
    // Publish events
    for i in 0..num_events {
        let event = create_static_event(i as u64);
        let _ = sender.send(Arc::new(event));
    }
    
    // Wait for events to be processed
    let expected_count = num_subscribers as u64 * num_events as u64;
    let mut attempts = 0;
    while counter.load(std::sync::atomic::Ordering::Relaxed) < expected_count && attempts < 100 {
        tokio::time::sleep(Duration::from_millis(10)).await;
        attempts += 1;
    }
    
    let elapsed = start.elapsed();
    
    // Clean up
    for task in tasks {
        task.abort();
    }
    
    (elapsed, counter.load(std::sync::atomic::Ordering::Relaxed))
}

#[tokio::main]
async fn main() {
    // Test parameters
    let subscribers = [10, 100];
    let num_events = 10000;
    
    println!("Starting API vs Direct benchmarks");
    println!("===============================");
    
    for &num_subscribers in &subscribers {
        println!("\nBenchmarking with {} subscribers, {} events per test", num_subscribers, num_events);
        
        // Run Zero Copy benchmarks
        let (api_zero_copy_time, api_zero_copy_count) = 
            bench_api_zero_copy(num_subscribers, num_events).await;
        
        let (direct_zero_copy_time, direct_zero_copy_count) = 
            bench_direct_zero_copy(num_subscribers, num_events).await;
        
        println!("\nZero Copy Results:");
        println!("  API:    {:?} ({} events, {:.2} events/sec)", 
                api_zero_copy_time, 
                api_zero_copy_count,
                api_zero_copy_count as f64 / api_zero_copy_time.as_secs_f64());
        
        println!("  Direct: {:?} ({} events, {:.2} events/sec)", 
                direct_zero_copy_time, 
                direct_zero_copy_count,
                direct_zero_copy_count as f64 / direct_zero_copy_time.as_secs_f64());
        
        println!("  Overhead: {:.2}%", 
                (api_zero_copy_time.as_secs_f64() / direct_zero_copy_time.as_secs_f64() - 1.0) * 100.0);
        
        // Give some time between tests
        tokio::time::sleep(Duration::from_secs(2)).await;
        
        // Run Static Fast Path benchmarks
        let (api_static_time, api_static_count) = 
            bench_api_static_fast_path(num_subscribers, num_events).await;
        
        let (direct_static_time, direct_static_count) = 
            bench_direct_static_fast_path(num_subscribers, num_events).await;
        
        println!("\nStatic Fast Path Results:");
        println!("  API:    {:?} ({} events, {:.2} events/sec)", 
                api_static_time, 
                api_static_count,
                api_static_count as f64 / api_static_time.as_secs_f64());
        
        println!("  Direct: {:?} ({} events, {:.2} events/sec)", 
                direct_static_time, 
                direct_static_count,
                direct_static_count as f64 / direct_static_time.as_secs_f64());
        
        println!("  Overhead: {:.2}%", 
                (api_static_time.as_secs_f64() / direct_static_time.as_secs_f64() - 1.0) * 100.0);
        
        // Give some time between tests
        tokio::time::sleep(Duration::from_secs(2)).await;
    }
} 