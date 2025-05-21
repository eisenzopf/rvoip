fn print_stats(&self) {
    let count = self.packets_processed.load(Ordering::Relaxed);
    let elapsed = self.start_time.elapsed().as_secs_f64();
    let rate = count as f64 / elapsed;
    
    println!("[{}] Processed {} packets ({:.2} packets/sec)",
        self.name,
        count,
        rate);
}

/// Demonstrate batch publishing
async fn demonstrate_batch_publishing(event_system: &EventSystem) -> EventResult<()> {
    println!("\nDemonstrating batch publishing...");
    
    // Create the publisher
    let publisher = event_system.create_publisher::<MediaPacketEvent>();
    
    // Create a subscriber
    let mut subscriber = event_system.subscribe::<MediaPacketEvent>().await?;
    
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
    while let Ok(Ok(event)) = tokio::time::timeout(
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