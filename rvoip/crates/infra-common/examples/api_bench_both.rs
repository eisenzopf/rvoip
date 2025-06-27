use infra_common::events::system::EventSystem;
use infra_common::events::types::{Event, EventPriority, EventResult, StaticEvent};
use infra_common::events::builder::{EventSystemBuilder, ImplementationType};
use infra_common::events::registry::GlobalTypeRegistry;
use infra_common::events::api::EventSystem as EventSystemTrait;
use infra_common::events::api::{EventPublisher, EventSubscriber};
use serde::{Serialize, Deserialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};
use std::time::{Duration, Instant};
use std::any::Any;

// ---- Constants for testing ----
const SUBSCRIBER_COUNT: usize = 5;
const TEST_DURATION_SECS: u64 = 30;
const CHANNEL_CAPACITY: usize = 10_000;
const DEBUG_MODE: bool = false;
const MINIMAL_OUTPUT: bool = true; // Set to true to show only essential results

/// Define a media packet event that's compatible with both implementations
#[derive(Clone, Debug, Serialize, Deserialize)]
struct MediaPacketEvent {
    stream_id: String,
    sequence_number: u32,
    timestamp: u64,
    payload_type: u8,
    marker: bool,
    payload_size: usize,
}

// Implement Event trait for MediaPacketEvent
impl Event for MediaPacketEvent {
    fn event_type() -> &'static str {
        "media_packet"
    }
    
    fn priority() -> EventPriority {
        EventPriority::High
    }
    
    fn as_any(&self) -> &dyn Any {
        self
    }
}

// Implement StaticEvent to enable fast path
impl StaticEvent for MediaPacketEvent {}

// Register our MediaPacketEvent with the GlobalTypeRegistry
fn register_event_types() {
    GlobalTypeRegistry::register_static_event_type::<MediaPacketEvent>();
    if DEBUG_MODE {
        println!("Registered MediaPacketEvent as StaticEvent");
    }
    
    // Also register with a specific capacity for better performance
    GlobalTypeRegistry::register_with_capacity::<MediaPacketEvent>(CHANNEL_CAPACITY);
    if DEBUG_MODE {
        println!("Configured MediaPacketEvent with capacity {}", CHANNEL_CAPACITY);
    }
    
    // Add a short delay to ensure the registration is fully processed
    std::thread::sleep(std::time::Duration::from_millis(50));
}

/// Stats collector for performance measurement
struct StatsCollector {
    name: String,
    packets_processed: AtomicU64,
    start_time: Instant,
}

impl StatsCollector {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            packets_processed: AtomicU64::new(0),
            start_time: Instant::now(),
        }
    }
    
    fn count_event(&self) {
        self.packets_processed.fetch_add(1, Ordering::Relaxed);
    }
    
    fn print_stats(&self) {
        let count = self.packets_processed.load(Ordering::Relaxed);
        let elapsed = self.start_time.elapsed().as_secs_f64();
        let rate = count as f64 / elapsed;
        
        if MINIMAL_OUTPUT {
            println!("{}: {} packets ({:.2} packets/sec)", 
                self.name, count, rate);
        } else {
            println!("[{}] Processed {} packets ({:.2} packets/sec)",
                self.name,
                count,
                rate);
        }
    }
}

/// Create a test media packet
fn create_media_packet(id: u64) -> MediaPacketEvent {
    MediaPacketEvent {
        stream_id: format!("stream-{}", id % 8),
        sequence_number: (id % 65535) as u32,
        timestamp: chrono::Utc::now().timestamp_millis() as u64,
        payload_type: 96,
        marker: id % 30 == 0,
        payload_size: 1400,
    }
}

/// Run a benchmark with the specified event system
async fn run_benchmark(
    event_system: EventSystem,
    implementation_name: &str,
) -> EventResult<()> {
    if !MINIMAL_OUTPUT {
        println!("\n{} Implementation Test", implementation_name);
        println!("==========================");
        println!("Subscribers: {}", SUBSCRIBER_COUNT);
        println!("Duration: {} seconds", TEST_DURATION_SECS);
        println!("Channel capacity: {}", CHANNEL_CAPACITY);
    } else {
        println!("\nRunning {} test...", implementation_name);
    }
    
    // Start the event system
    EventSystemTrait::start(&event_system).await?;
    
    // Register MediaPacketEvent to ensure it's available
    GlobalTypeRegistry::register_static_event_type::<MediaPacketEvent>();
    GlobalTypeRegistry::register_with_capacity::<MediaPacketEvent>(CHANNEL_CAPACITY);
    
    // Create the publisher
    let publisher = EventSystemTrait::create_publisher::<MediaPacketEvent>(&event_system);
    
    // Create the stats collector
    let stats = Arc::new(StatsCollector::new(implementation_name));
    
    // Create and start subscribers first, before publishing
    let mut handles = Vec::new();
    let implementation_name_str = implementation_name.to_string();
    
    // Create a barrier to synchronize the start of subscribers
    let (start_tx, _) = tokio::sync::broadcast::channel::<bool>(SUBSCRIBER_COUNT);
    
    // Create all subscribers
    for i in 0..SUBSCRIBER_COUNT {
        let mut subscriber = EventSystemTrait::subscribe::<MediaPacketEvent>(&event_system).await?;
        let stats_clone = stats.clone();
        let impl_name_clone = implementation_name_str.clone();
        let mut rx = start_tx.subscribe();
        
        let handle = tokio::spawn(async move {
            // Wait for the start signal
            let _ = rx.recv().await;
            
            // Process events
            let mut count = 0;
            while let Ok(event) = subscriber.receive().await {
                count += 1;
                stats_clone.count_event();
                
                // Minimal logging for important events
                if !MINIMAL_OUTPUT && count <= 5 {
                    println!("Subscriber {} received event {}: stream_id={}, seq={}", 
                        i, count, event.stream_id, event.sequence_number);
                }
            }
        });
        
        handles.push(handle);
    }
    
    // Small pause before starting publisher to ensure all subscribers are ready
    tokio::time::sleep(Duration::from_millis(200)).await;
    
    // Signal all subscribers to start processing
    let _ = start_tx.send(true);
    
    // Another small pause to ensure subscribers are ready
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    // Start publishing task
    let publisher_handle = tokio::spawn({
        let publisher = publisher;
        
        async move {
            let mut event_id = 0;
            let end_time = Instant::now() + Duration::from_secs(TEST_DURATION_SECS);
            
            while Instant::now() < end_time {
                let packet = create_media_packet(event_id);
                
                // Publish event and handle any errors
                let result = publisher.publish(packet).await;
                if let Err(e) = result {
                    if !MINIMAL_OUTPUT {
                        println!("Publish error: {}", e);
                    }
                    tokio::time::sleep(Duration::from_millis(1)).await;
                }
                
                event_id += 1;
                
                // Yield periodically to avoid starving other tasks
                if event_id % 1000 == 0 {
                    tokio::task::yield_now().await;
                }
            }
        }
    });
    
    // Wait for publisher to finish
    publisher_handle.await.unwrap();
    
    // Let subscribers process any remaining events
    tokio::time::sleep(Duration::from_millis(500)).await;
    
    // Print final stats
    stats.print_stats();
    
    // Shutdown the event system
    EventSystemTrait::shutdown(&event_system).await?;
    
    // Cancel all subscriber tasks
    for handle in handles {
        handle.abort();
    }
    
    Ok(())
}

/// Demonstrate batch publishing
async fn demonstrate_batch_publishing(event_system: &EventSystem) -> EventResult<()> {
    println!("\nDemonstrating batch publishing...");
    
    // Create the publisher
    let publisher = EventSystemTrait::create_publisher::<MediaPacketEvent>(event_system);
    
    // Create a subscriber
    let mut subscriber = EventSystemTrait::subscribe::<MediaPacketEvent>(event_system).await?;
    
    // Prepare a batch of events
    let batch_size = 100;
    let events: Vec<MediaPacketEvent> = (0..batch_size)
        .map(|i| create_media_packet(i as u64))
        .collect();
    
    println!("Publishing batch of {} events", batch_size);
    
    // Publish the batch
    publisher.publish_batch(events).await?;
    
    // Receive some events to verify
    let mut received = 0;
    while let Ok(Ok(_event)) = tokio::time::timeout(
        Duration::from_millis(100),
        subscriber.receive()
    ).await {
        received += 1;
        if received >= 5 {
            break;
        }
    }
    
    println!("Successfully received {} events from batch", received);
    Ok(())
}

/// Simple test for the StaticEvent publishing
async fn test_static_event_publishing() -> EventResult<()> {
    println!("\nSimple StaticEvent Publishing Test");
    println!("-------------------------------");
    
    // Create a static system with enough capacity
    let system = EventSystemBuilder::new()
        .implementation(ImplementationType::StaticFastPath)
        .channel_capacity(1000)
        .build();
    
    // Start the system
    EventSystemTrait::start(&system).await?;
    
    // Create a publisher
    let publisher = EventSystemTrait::create_publisher::<MediaPacketEvent>(&system);
    
    // Subscribe to the events
    let mut subscriber = EventSystemTrait::subscribe::<MediaPacketEvent>(&system).await?;
    
    // Create a test packet
    let packet = create_media_packet(42);
    
    // Publish it
    println!("Publishing a single packet...");
    publisher.publish(packet.clone()).await?;
    
    // Try to receive it with a timeout
    match tokio::time::timeout(Duration::from_secs(1), subscriber.receive()).await {
        Ok(Ok(received)) => {
            println!("Successfully received packet with sequence number: {}", received.sequence_number);
            println!("Packet details: stream_id={}, marker={}", received.stream_id, received.marker);
        },
        Ok(Err(e)) => {
            println!("Error receiving packet: {}", e);
        },
        Err(_) => {
            println!("Timeout waiting for packet");
        }
    }
    
    // Shut down the system
    EventSystemTrait::shutdown(&system).await?;
    
    Ok(())
}

fn main() -> Result<(), Box<dyn std::error::Error>> {
    if MINIMAL_OUTPUT {
        println!("Running event system performance comparison:");
        println!("- {} subscribers, {} seconds per test", SUBSCRIBER_COUNT, TEST_DURATION_SECS);
    } else {
        println!("Unified Event System API - Performance Benchmark");
        println!("===============================================");
    }
    
    // Register our event types first
    register_event_types();
    
    // Create a static fast path event system
    let static_system = EventSystemBuilder::new()
        .implementation(ImplementationType::StaticFastPath)
        .channel_capacity(CHANNEL_CAPACITY)
        .build();
    
    // Create a zero-copy event bus
    let zero_copy_system = EventSystemBuilder::new()
        .implementation(ImplementationType::ZeroCopy)
        .channel_capacity(CHANNEL_CAPACITY)
        .max_concurrent_dispatches(1000)
        .enable_priority(true)
        .default_timeout(Some(Duration::from_secs(1)))
        .shard_count(10)
        .enable_metrics(true)
        .metrics_reporting_interval(Duration::from_secs(5))
        .build();
    
    // Create tokio runtime for running the benchmarks
    let runtime = tokio::runtime::Builder::new_multi_thread()
        .worker_threads(4)
        .enable_all()
        .build()?;
    
    // Run benchmarks for each implementation
    runtime.block_on(async {
        // Short introduction
        if !MINIMAL_OUTPUT {
            println!("Running performance tests for static fast path and zero copy implementations");
            println!("Each test will run for {} seconds with {} subscribers", TEST_DURATION_SECS, SUBSCRIBER_COUNT);
        }
        
        // Run benchmark for static fast path implementation
        run_benchmark(static_system, "Static Fast Path").await?;
        
        // Short pause between benchmarks
        tokio::time::sleep(Duration::from_secs(2)).await;
        
        // Run benchmark for zero-copy implementation
        run_benchmark(zero_copy_system, "Zero Copy").await?;
                
        Ok::<(), Box<dyn std::error::Error>>(())
    })?;
    
    if !MINIMAL_OUTPUT {
        println!("\nAll benchmarks completed successfully");
    } else {
        println!("\nTest complete.");
    }
    Ok(())
}

/// Example component that can use either implementation
struct MediaProcessor {
    event_system: EventSystem,
}

impl MediaProcessor {
    pub fn new(high_performance: bool) -> Self {
        let builder = EventSystemBuilder::new()
            .channel_capacity(CHANNEL_CAPACITY);
        
        let event_system = if high_performance {
            // Use static fast path for max performance
            builder.implementation(ImplementationType::StaticFastPath)
        } else {
            // Use zero-copy for more features
            builder.implementation(ImplementationType::ZeroCopy)
                .max_concurrent_dispatches(500)
                .enable_priority(true)
                .default_timeout(Some(Duration::from_millis(100)))
        }.build();
        
        Self { event_system }
    }
    
    pub async fn start(&self) -> EventResult<()> {
        // Start the event system
        EventSystemTrait::start(&self.event_system).await?;
        
        // Report which implementation we're using
        if let Some(_) = self.event_system.advanced() {
            println!("Started media processor with feature-rich zero-copy implementation");
        } else {
            println!("Started media processor with high-performance static implementation");
        }
        
        Ok(())
    }
} 