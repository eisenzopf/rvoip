use rvoip_infra_common::events::bus::{EventBus, EventBusConfig};
use rvoip_infra_common::events::types::{Event, EventPriority, EventHandler};
use async_trait::async_trait;
use serde::{Serialize, Deserialize};
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, AtomicBool, Ordering};
use std::time::{Duration, Instant};
use std::any::Any;

// ---- Constants for testing ----
const SUBSCRIBER_COUNT: usize = 20;
const PUBLISHER_COUNT: usize = 5;
const TEST_DURATION_SECS: u64 = 30;
const WARMUP_EVENTS: usize = 10_000;
const CHANNEL_CAPACITY: usize = 10_000; // Match static event example

/// Example: Define an event
#[derive(Clone, Debug, Serialize, Deserialize)]
struct MediaPacketEvent {
    stream_id: String,
    sequence_number: u32,
    timestamp: u64,
    payload_type: u8,
    marker: bool,
    payload_size: usize,
}

impl Event for MediaPacketEvent {
    fn event_type() -> &'static str {
        "media_packet"
    }
    
    fn priority() -> EventPriority {
        // Media packets need fast processing
        EventPriority::High
    }
    
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Example: Media packet processor
#[derive(Clone)]
struct MediaProcessor {
    name: String,
    packets_processed: Arc<AtomicU64>,
}

impl MediaProcessor {
    fn new(name: &str) -> Self {
        Self {
            name: name.to_string(),
            packets_processed: Arc::new(AtomicU64::new(0)),
        }
    }
    
    fn print_stats(&self) {
        println!("[{}] Processed {} packets",
            self.name,
            self.packets_processed.load(Ordering::Relaxed));
    }
}

#[async_trait]
impl EventHandler<MediaPacketEvent> for MediaProcessor {
    async fn handle(&self, _event: MediaPacketEvent) {
        // Just count packets, no printing to avoid affecting performance
        self.packets_processed.fetch_add(1, Ordering::Relaxed);
    }
}

/// Create a media packet event
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

#[tokio::main]
async fn main() -> Result<(), Box<dyn std::error::Error>> {
    println!("Zero Copy Event Bus Performance Test");
    println!("===================================");
    println!("Configuration:");
    println!(" - {} subscribers", SUBSCRIBER_COUNT);
    println!(" - {} publishers", PUBLISHER_COUNT);
    println!(" - {} second test duration", TEST_DURATION_SECS);
    println!(" - {} channel capacity\n", CHANNEL_CAPACITY);
    
    // Create an event bus with zero copy configuration
    let event_bus = EventBus::with_config(EventBusConfig {
        max_concurrent_dispatches: 1000,
        default_timeout: Duration::from_secs(1),
        broadcast_capacity: CHANNEL_CAPACITY,  // Use the constant
        enable_priority: true,
        enable_zero_copy: true,
        batch_size: 100,
        shard_count: 8,
    });
    
    // Create and register handlers
    let processors: Vec<MediaProcessor> = (0..SUBSCRIBER_COUNT)
        .map(|i| MediaProcessor::new(&format!("Processor{}", i+1)))
        .collect();
    
    // Register all processors
    for processor in &processors {
        event_bus.subscribe::<MediaPacketEvent, _>(None, processor.clone()).await?;
    }
    
    // Warmup phase
    println!("Running warmup phase...");
    for i in 0..WARMUP_EVENTS {
        event_bus.publish(create_media_packet(i as u64)).await?;
    }
    tokio::time::sleep(Duration::from_millis(100)).await;
    
    // High throughput measurement
    println!("Starting performance test for {} seconds...", TEST_DURATION_SECS);
    
    let start = Instant::now();
    let end_time = start + Duration::from_secs(TEST_DURATION_SECS);
    
    // Shared event counter
    let events_published = Arc::new(AtomicU64::new(0));
    // Shared flag to stop publishing
    let stop_publishing = Arc::new(AtomicBool::new(false));
    
    // Create multiple publisher tasks
    let publisher_tasks: Vec<_> = (0..PUBLISHER_COUNT).map(|publisher_id| {
        let event_bus = event_bus.clone();
        let events_counter = events_published.clone();
        let stop_flag = stop_publishing.clone();
        
        tokio::spawn(async move {
            let mut local_counter: u64 = 0;
            let mut event_id = publisher_id as u64 * 1_000_000; // Ensure unique IDs
            
            while !stop_flag.load(Ordering::Relaxed) {
                let _ = event_bus.publish(create_media_packet(event_id)).await;
                event_id += 1;
                local_counter += 1;
                
                if local_counter % 10_000 == 0 {
                    events_counter.fetch_add(local_counter, Ordering::Relaxed);
                    local_counter = 0;
                    
                    // Give other tasks a chance to run
                    tokio::task::yield_now().await;
                }
            }
            
            // Add any remaining events
            events_counter.fetch_add(local_counter, Ordering::Relaxed);
        })
    }).collect();
    
    // Monitor task to report progress and stop publishers after duration
    let (progress_tx, mut progress_rx) = tokio::sync::mpsc::channel::<()>(1);
    tokio::spawn({
        let events_counter = events_published.clone();
        let stop_flag = stop_publishing.clone();
        async move {
            let mut last_count = 0;
            let mut last_time = Instant::now();
            
            while Instant::now() < end_time {
                tokio::time::sleep(Duration::from_secs(5)).await;
                let now = Instant::now();
                let elapsed = now.duration_since(last_time);
                let current_count = events_counter.load(Ordering::Relaxed);
                let new_events = current_count - last_count;
                let rate = new_events as f64 / elapsed.as_secs_f64();
                
                println!("Progress: {:.2} events/second", rate);
                
                last_count = current_count;
                last_time = now;
            }
            
            // Test duration reached, signal publishers to stop
            stop_flag.store(true, Ordering::Relaxed);
            let _ = progress_tx.send(()).await;
        }
    });
    
    // Wait for test duration to complete
    let _ = progress_rx.recv().await;
    
    // Wait for all publishers to complete
    for task in publisher_tasks {
        let _ = task.await;
    }
    
    // Get final event count
    let total_events = events_published.load(Ordering::Relaxed);
    let publish_elapsed = start.elapsed();
    println!("Test completed in {:?}", publish_elapsed);
    println!("Total events published: {}", total_events);
    println!("Average publishing throughput: {:.2} events/second", 
             total_events as f64 / publish_elapsed.as_secs_f64());
    
    // Wait for events to be processed
    let processing_time = Duration::from_secs(1);
    println!("Waiting {} seconds for event processing...", processing_time.as_secs());
    tokio::time::sleep(processing_time).await;
    
    // Calculate event processing statistics
    let total_processed: u64 = processors.iter()
        .map(|p| p.packets_processed.load(Ordering::Relaxed))
        .sum();
    
    println!("\nProcessing Statistics:");
    println!("Total events published: {}", total_events);
    println!("Total events processed: {}", total_processed);
    println!("Processing ratio: {:.2}%", (total_processed as f64 / total_events as f64) * 100.0);
    
    if SUBSCRIBER_COUNT <= 5 {
        // Print individual stats for a small number of subscribers
        for processor in &processors {
            processor.print_stats();
        }
    } else {
        // Just show average for large number of subscribers
        let avg_per_subscriber = total_processed as f64 / SUBSCRIBER_COUNT as f64;
        println!("Average events per subscriber: {:.2}", avg_per_subscriber);
    }
    
    println!("\nTest complete!");
    Ok(())
} 