use criterion::{black_box, criterion_group, criterion_main, Criterion, BenchmarkId, Throughput};
use infra_common::events::bus::{EventBus, EventBusConfig};
use infra_common::events::types::{Event, EventHandler, EventPriority, StaticEvent};
use infra_common::events::publisher::{Publisher, FastPublisher};
use infra_common::events::registry::GlobalTypeRegistry;
use async_trait::async_trait;
use std::sync::Arc;
use std::time::Duration;
use tokio::runtime::Runtime;
use serde::{Serialize, Deserialize};

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

#[async_trait]
impl EventHandler<StaticTestEvent> for TestHandler {
    async fn handle(&self, _event: StaticTestEvent) {
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

fn create_static_event(id: u64) -> StaticTestEvent {
    StaticTestEvent {
        id,
        data: format!("Static event data for event {}", id),
    }
}

fn bench_event_bus_single_publisher(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    
    let mut group = c.benchmark_group("event_bus_single_publisher");
    
    for &num_subscribers in &[1, 10, 100] {
        group.throughput(Throughput::Elements(100));
        group.bench_with_input(
            BenchmarkId::from_parameter(num_subscribers),
            &num_subscribers,
            |b, &n| {
                b.iter(|| {
                    rt.block_on(async {
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
                        
                        // Register subscribers
                        for _ in 0..n {
                            let handler = TestHandler { counter: counter.clone() };
                            let _ = event_bus.subscribe::<TestEvent, _>(None, handler).await.unwrap();
                        }
                        
                        // Publish events using direct event bus, ignoring errors
                        for i in 0..100 {
                            let _ = event_bus.publish(create_test_event(i, EventPriority::Normal)).await;
                        }
                        
                        // Wait for all events to be processed
                        let expected_count = n * 100;
                        let mut attempts = 0;
                        while counter.load(std::sync::atomic::Ordering::Relaxed) < expected_count && attempts < 100 {
                            tokio::time::sleep(Duration::from_millis(10)).await;
                            attempts += 1;
                        }
                        
                        assert_eq!(counter.load(std::sync::atomic::Ordering::Relaxed), expected_count);
                    });
                });
            },
        );
    }
    
    group.finish();
}

fn bench_event_bus_channel(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    
    let mut group = c.benchmark_group("event_bus_channel");
    
    // Configure longer measurement time for high-volume tests
    group.measurement_time(Duration::from_secs(10));
    
    for &num_subscribers in &[10, 100, 1000] {
        group.throughput(Throughput::Elements(1000));
        group.bench_with_input(
            BenchmarkId::from_parameter(num_subscribers),
            &num_subscribers,
            |b, &n| {
                b.iter(|| {
                    rt.block_on(async {
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
                        let dummy_handler = TestHandler { counter: counter.clone() };
                        let _ = event_bus.subscribe::<TestEvent, _>(None, dummy_handler).await.unwrap();
                        
                        // Publish one event to ensure the broadcast channel is created
                        let _ = event_bus.publish(create_test_event(0, EventPriority::Normal)).await;
                        
                        // Create channel subscribers
                        let mut receivers = Vec::new();
                        for _ in 0..n {
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
                        let expected_count = n * 1000;
                        let mut attempts = 0;
                        while counter.load(std::sync::atomic::Ordering::Relaxed) < expected_count && attempts < 100 {
                            tokio::time::sleep(Duration::from_millis(10)).await;
                            attempts += 1;
                        }

                        if counter.load(std::sync::atomic::Ordering::Relaxed) < expected_count {
                            println!("Warning: Not all events were processed. Got {} out of {}", 
                                counter.load(std::sync::atomic::Ordering::Relaxed), expected_count);
                        }
                        
                        // Explicitly drop the counter tasks to avoid task leaks
                        for task in counter_tasks {
                            task.abort();
                        }
                    });
                });
            },
        );
    }
    
    group.finish();
}

fn bench_event_priority(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    
    let mut group = c.benchmark_group("event_priority");
    
    for &priority in &[EventPriority::Low, EventPriority::Normal, EventPriority::High, EventPriority::Critical] {
        group.throughput(Throughput::Elements(100));
        group.bench_with_input(
            BenchmarkId::from_parameter(format!("{:?}", priority)),
            &priority,
            |b, &p| {
                b.iter(|| {
                    rt.block_on(async {
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
                        
                        // Register subscribers with different priorities
                        for i in 0..10 {
                            let handler = TestHandler { counter: counter.clone() };
                            let _priority = match i % 4 {
                                0 => EventPriority::Low,
                                1 => EventPriority::Normal,
                                2 => EventPriority::High,
                                _ => EventPriority::Critical,
                            };
                            let _ = event_bus.subscribe::<TestEvent, _>(None, handler).await.unwrap();
                        }
                        
                        // Publish events with the specified priority
                        for i in 0..100 {
                            let _ = event_bus.publish(create_test_event(i, p)).await;
                        }
                        
                        // Wait for all events to be processed
                        let expected_count = 10 * 100;
                        let mut attempts = 0;
                        while counter.load(std::sync::atomic::Ordering::Relaxed) < expected_count && attempts < 100 {
                            tokio::time::sleep(Duration::from_millis(10)).await;
                            attempts += 1;
                        }
                    });
                });
            },
        );
    }
    
    group.finish();
}

fn bench_zero_copy_event_bus(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    
    let mut group = c.benchmark_group("zero_copy_event_bus");
    
    // Configure longer measurement time for high-volume tests
    group.measurement_time(Duration::from_secs(15));
    
    for &num_subscribers in &[1, 10, 100, 1000] {
        group.throughput(Throughput::Elements(1000));
        group.bench_with_input(
            BenchmarkId::from_parameter(num_subscribers),
            &num_subscribers,
            |b, &n| {
                b.iter(|| {
                    rt.block_on(async {
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
                        for _ in 0..n {
                            let rx = event_bus.subscribe_broadcast::<TestEvent>().await.unwrap();
                            receivers.push(rx);
                        }
                        
                        // Spawn tasks to count received events
                        let tasks: Vec<_> = receivers.into_iter().map(|mut rx| {
                            let counter = counter.clone();
                            tokio::spawn(async move {
                                while let Ok(event) = rx.recv().await {
                                    let _ = event; // Use event to prevent the compiler from optimizing it away
                                    counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                }
                            })
                        }).collect();
                        
                        // Publish events
                        for i in 0..1000 {
                            let _ = event_bus.publish(create_test_event(i, EventPriority::Normal)).await;
                        }
                        
                        // Wait for events to be processed
                        let expected_count = n * 1000;
                        let mut attempts = 0;
                        while counter.load(std::sync::atomic::Ordering::Relaxed) < expected_count && attempts < 100 {
                            tokio::time::sleep(Duration::from_millis(10)).await;
                            attempts += 1;
                        }
                        
                        for task in tasks {
                            task.abort();
                        }
                    });
                });
            },
        );
    }
    
    group.finish();
}

fn bench_static_event_fast_path(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    
    let mut group = c.benchmark_group("static_event_fast_path");
    
    // Configure longer measurement time for high-volume tests
    group.measurement_time(Duration::from_secs(20));
    group.sample_size(80);
    
    for &num_subscribers in &[10, 100, 1000] {
        group.throughput(Throughput::Elements(10000));
        group.bench_with_input(
            BenchmarkId::from_parameter(num_subscribers),
            &num_subscribers,
            |b, &n| {
                b.iter(|| {
                    rt.block_on(async {
                        // Create a fast publisher for static events
                        let publisher = FastPublisher::<StaticTestEvent>::new();
                        
                        let counter = Arc::new(std::sync::atomic::AtomicU64::new(0));
                        
                        // Create subscribers via global registry
                        let tasks: Vec<_> = (0..n).map(|_| {
                            let counter = counter.clone();
                            let mut rx = publisher.subscribe();
                            tokio::spawn(async move {
                                while let Ok(event) = rx.recv().await {
                                    let _ = event; // Use event to prevent compiler optimization
                                    counter.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                                }
                            })
                        }).collect();
                        
                        // Publish a large number of events
                        for i in 0..10000 {
                            let _ = publisher.publish(create_static_event(i)).await;
                        }
                        
                        // Wait for events to be processed
                        let expected_count = n * 10000;
                        let mut attempts = 0;
                        while counter.load(std::sync::atomic::Ordering::Relaxed) < expected_count && attempts < 100 {
                            tokio::time::sleep(Duration::from_millis(10)).await;
                            attempts += 1;
                        }
                        
                        for task in tasks {
                            task.abort();
                        }
                    });
                });
            },
        );
    }
    
    group.finish();
}

fn bench_batch_processing(c: &mut Criterion) {
    let rt = Runtime::new().unwrap();
    
    let mut group = c.benchmark_group("batch_processing");
    
    // Configure longer measurement time for high-volume tests
    group.measurement_time(Duration::from_secs(10));
    
    for &batch_size in &[10, 100, 1000] {
        group.throughput(Throughput::Elements(10000));
        group.bench_with_input(
            BenchmarkId::from_parameter(batch_size),
            &batch_size,
            |b, &size| {
                b.iter(|| {
                    rt.block_on(async {
                        // Setup event bus with batch settings
                        let event_bus = EventBus::with_config(EventBusConfig {
                            max_concurrent_dispatches: 10000,
                            default_timeout: Duration::from_secs(1),
                            shard_count: 32,
                            broadcast_capacity: 4096,
                            enable_priority: true,
                            enable_zero_copy: true,
                            batch_size: size,
                        });
                        
                        let counter = Arc::new(std::sync::atomic::AtomicU64::new(0));
                        let publisher = Publisher::<TestEvent>::new(event_bus.clone());
                        
                        // Subscribe to events
                        let mut rx = event_bus.subscribe_broadcast::<TestEvent>().await.unwrap();
                        let counter_clone = counter.clone();
                        let task = tokio::spawn(async move {
                            while let Ok(event) = rx.recv().await {
                                let _ = event; // Use event to prevent compiler optimization
                                counter_clone.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                            }
                        });
                        
                        // Create batch of events
                        let batches = 10000 / size;
                        for j in 0..batches {
                            let mut batch = Vec::with_capacity(size);
                            for i in 0..size {
                                batch.push(create_test_event((j * size + i) as u64, EventPriority::Normal));
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
                        
                        task.abort();
                    });
                });
            },
        );
    }
    
    group.finish();
}

criterion_group!(
    benches,
    bench_event_bus_single_publisher,
    bench_event_bus_channel,
    bench_event_priority,
    bench_zero_copy_event_bus,
    bench_static_event_fast_path,
    bench_batch_processing
);
criterion_main!(benches);

#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_event_bus_broadcast() {
        let rt = Runtime::new().unwrap();
        
        rt.block_on(async {
            // Setup
            let event_bus = EventBus::new();
            let counter = Arc::new(std::sync::atomic::AtomicU64::new(0));
            
            // Register a subscriber
            let handler = TestHandler { counter: counter.clone() };
            let _ = event_bus.subscribe::<TestEvent, _>(None, handler).await.unwrap();
            
            // Get a broadcast channel
            let mut rx = event_bus.subscribe_broadcast::<TestEvent>().await.unwrap();
            
            // Spawn a task to process broadcast events
            let channel_counter = Arc::new(std::sync::atomic::AtomicU64::new(0));
            let channel_counter_clone = channel_counter.clone();
            let task = tokio::spawn(async move {
                while let Ok(_) = rx.recv().await {
                    channel_counter_clone.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            });
            
            // We should now have two ways to receive events
            let _ = event_bus.publish(create_test_event(1, EventPriority::Normal)).await;
            
            // Wait a bit for processing
            tokio::time::sleep(Duration::from_millis(100)).await;
            
            // Both subscribers should have received it
            assert_eq!(counter.load(std::sync::atomic::Ordering::Relaxed), 1, 
                      "Direct subscriber should have received the event");
            assert_eq!(channel_counter.load(std::sync::atomic::Ordering::Relaxed), 1,
                      "Broadcast channel should have received the event");
            
            // Clean up
            task.abort();
        });
    }
    
    #[test]
    fn test_fast_publisher() {
        let rt = Runtime::new().unwrap();
        
        rt.block_on(async {
            // Create a fast publisher for static events
            let publisher = FastPublisher::<StaticTestEvent>::new();
            
            // Subscribe to events
            let mut rx = publisher.subscribe();
            let counter = Arc::new(std::sync::atomic::AtomicU64::new(0));
            let counter_clone = counter.clone();
            let task = tokio::spawn(async move {
                while let Ok(_) = rx.recv().await {
                    counter_clone.fetch_add(1, std::sync::atomic::Ordering::Relaxed);
                }
            });
            
            // Publish events
            for i in 0..10 {
                let _ = publisher.publish(create_static_event(i)).await;
            }
            
            // Wait for events to be processed
            tokio::time::sleep(Duration::from_millis(100)).await;
            
            // Subscribers should have received the events
            assert_eq!(counter.load(std::sync::atomic::Ordering::Relaxed), 10,
                      "Fast publisher receiver should have received 10 events");
            
            // Clean up
            task.abort();
        });
    }
} 