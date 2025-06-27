use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId, Throughput};
use infra_common::events::api::{EventSystem, EventPublisher, EventSubscriber};
use infra_common::events::builder::{EventSystemBuilder, ImplementationType};
use infra_common::events::types::{Event, EventPriority, StaticEvent};
use infra_common::events::registry::GlobalTypeRegistry;
use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;
use tokio::runtime::Runtime;
use serde::{Serialize, Deserialize};

// --- Common benchmark configuration ---
const CHANNEL_CAPACITY: usize = 10000;
const EVENT_COUNT: usize = 10000;
const BENCH_MEASUREMENT_TIME: Duration = Duration::from_secs(20);
const BENCH_SAMPLE_SIZE: usize = 80;
const SHARD_COUNT: usize = 32;
const SUBSCRIBER_COUNTS: [usize; 4] = [1, 10, 100, 1000];

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

/// Benchmarks the Zero Copy implementation using the public API
fn bench_api_zero_copy(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    
    let mut group = c.benchmark_group("api_zero_copy");
    
    // Configure measurement parameters
    group.measurement_time(BENCH_MEASUREMENT_TIME);
    group.sample_size(BENCH_SAMPLE_SIZE);
    
    for &num_subscribers in &SUBSCRIBER_COUNTS {
        group.throughput(Throughput::Elements(EVENT_COUNT as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(num_subscribers),
            &num_subscribers,
            |b, &n| {
                b.iter(|| {
                    rt.block_on(async {
                        // Setup event system with zero-copy enabled
                        let system = EventSystemBuilder::new()
                            .implementation(ImplementationType::ZeroCopy)
                            .channel_capacity(CHANNEL_CAPACITY)
                            .max_concurrent_dispatches(10000)
                            .enable_priority(true)
                            .default_timeout(Some(Duration::from_secs(1)))
                            .batch_size(100)
                            .shard_count(SHARD_COUNT)
                            .build();
                        
                        // Start the event system
                        system.start().await.unwrap();
                        
                        let counter = Arc::new(std::sync::atomic::AtomicU64::new(0));
                        
                        // Create subscribers
                        let mut subscribers = Vec::new();
                        for _ in 0..n {
                            let subscriber = system.subscribe::<TestEvent>().await.unwrap();
                            subscribers.push(subscriber);
                        }
                        
                        // Spawn tasks to count received events
                        let tasks: Vec<_> = subscribers.into_iter().map(|mut subscriber| {
                            let counter = counter.clone();
                            tokio::spawn(async move {
                                while let Ok(event) = subscriber.receive_timeout(Duration::from_secs(2)).await {
                                    let _ = black_box(event); // Prevent compiler optimization
                                    counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                }
                            })
                        }).collect();
                        
                        // Create publisher
                        let publisher = system.create_publisher::<TestEvent>();
                        
                        // Publish events
                        for i in 0..EVENT_COUNT {
                            let event = create_test_event(i as u64, EventPriority::Normal);
                            let _ = publisher.publish(event).await;
                        }
                        
                        // Wait for events to be processed
                        let expected_count = (n * EVENT_COUNT) as u64;
                        let mut attempts = 0;
                        while counter.load(std::sync::atomic::Ordering::Relaxed) < expected_count && attempts < 100 {
                            tokio::time::sleep(Duration::from_millis(10)).await;
                            attempts += 1;
                        }
                        
                        // Clean up
                        for task in tasks {
                            task.abort();
                        }
                        
                        system.shutdown().await.unwrap();
                    });
                });
            },
        );
    }
    
    group.finish();
}

/// Benchmarks the Static Fast Path implementation using the public API
fn bench_api_static_fast_path(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    
    let mut group = c.benchmark_group("api_static_fast_path");
    
    // Register the static event type with the global registry
    rt.block_on(async {
        GlobalTypeRegistry::register_static_event_type::<StaticTestEvent>();
        GlobalTypeRegistry::register_with_capacity::<StaticTestEvent>(CHANNEL_CAPACITY);
    });
    
    // Configure measurement parameters (same as zero copy)
    group.measurement_time(BENCH_MEASUREMENT_TIME);
    group.sample_size(BENCH_SAMPLE_SIZE);
    
    for &num_subscribers in &SUBSCRIBER_COUNTS {
        group.throughput(Throughput::Elements(EVENT_COUNT as u64));
        group.bench_with_input(
            BenchmarkId::from_parameter(num_subscribers),
            &num_subscribers,
            |b, &n| {
                b.iter(|| {
                    rt.block_on(async {
                        // Create a static fast path event system with the same configuration as zero copy
                        // (where applicable)
                        let system = EventSystemBuilder::new()
                            .implementation(ImplementationType::StaticFastPath)
                            .channel_capacity(CHANNEL_CAPACITY)
                            .shard_count(SHARD_COUNT)  // Applying the same shard count as Zero Copy
                            .build();
                        
                        // Start the event system
                        system.start().await.unwrap();
                        
                        let counter = Arc::new(std::sync::atomic::AtomicU64::new(0));
                        
                        // Create subscribers
                        let mut subscribers = Vec::new();
                        for _ in 0..n {
                            let subscriber = system.subscribe::<StaticTestEvent>().await.unwrap();
                            subscribers.push(subscriber);
                        }
                        
                        // Spawn tasks to count received events
                        let tasks: Vec<_> = subscribers.into_iter().map(|mut subscriber| {
                            let counter = counter.clone();
                            tokio::spawn(async move {
                                while let Ok(event) = subscriber.receive_timeout(Duration::from_secs(2)).await {
                                    let _ = black_box(event); // Prevent compiler optimization
                                    counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                }
                            })
                        }).collect();
                        
                        // Create publisher and publish events
                        let publisher = system.create_publisher::<StaticTestEvent>();
                        
                        // Publish events (same count as zero copy)
                        for i in 0..EVENT_COUNT {
                            let event = create_static_event(i as u64);
                            let _ = publisher.publish(event).await;
                        }
                        
                        // Wait for events to be processed
                        let expected_count = (n * EVENT_COUNT) as u64;
                        let mut attempts = 0;
                        while counter.load(std::sync::atomic::Ordering::Relaxed) < expected_count && attempts < 100 {
                            tokio::time::sleep(Duration::from_millis(10)).await;
                            attempts += 1;
                        }
                        
                        // Clean up
                        for task in tasks {
                            task.abort();
                        }
                        
                        system.shutdown().await.unwrap();
                    });
                });
            },
        );
    }
    
    group.finish();
}

criterion_group!(
    benches,
    bench_api_zero_copy,
    bench_api_static_fast_path
);
criterion_main!(benches); 