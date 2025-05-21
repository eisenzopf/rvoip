//! # Event System
//! 
//! A high-performance, flexible event system for publish-subscribe communication patterns.
//! 
//! This module provides an event publishing and subscription system with multiple
//! implementations optimized for different use cases. Based on our benchmarks,
//! the Zero Copy implementation offers the best performance and is recommended for most uses.
//! 
//! ## Quick Start - Zero Copy Implementation
//! 
//! ```rust,no_run
//! use infra_common::events::system::EventSystem;
//! use infra_common::events::builder::{EventSystemBuilder, ImplementationType};
//! use infra_common::events::types::{Event, EventPriority};
//! use infra_common::events::api::EventSystem as EventSystemTrait;
//! use std::any::Any;
//! use std::time::Duration;
//! use serde::{Serialize, Deserialize};
//! 
//! // 1. Define your event type
//! #[derive(Clone, Debug, Serialize, Deserialize)]
//! struct MyEvent {
//!     id: u32,
//!     message: String,
//! }
//! 
//! // 2. Implement the Event trait
//! impl Event for MyEvent {
//!     fn event_type() -> &'static str {
//!         "my_event"
//!     }
//!     
//!     fn priority() -> EventPriority {
//!         EventPriority::Normal
//!     }
//!     
//!     fn as_any(&self) -> &dyn Any {
//!         self
//!     }
//! }
//! 
//! async fn example() -> Result<(), Box<dyn std::error::Error>> {
//!     // 3. Create a Zero Copy event system
//!     let system = EventSystemBuilder::new()
//!         .implementation(ImplementationType::ZeroCopy)
//!         .channel_capacity(1000)
//!         .max_concurrent_dispatches(500)
//!         .enable_priority(true)
//!         .shard_count(8)
//!         .build();
//!     
//!     // 4. Start the event system
//!     system.start().await?;
//!     
//!     // 5. Subscribe to events
//!     let mut subscriber = system.subscribe::<MyEvent>().await?;
//!     
//!     // 6. Create a publisher
//!     let publisher = system.create_publisher::<MyEvent>();
//!     
//!     // 7. Publish an event
//!     let event = MyEvent {
//!         id: 1,
//!         message: "Hello, World!".to_string(),
//!     };
//!     publisher.publish(event).await?;
//!     
//!     // 8. Receive an event
//!     match subscriber.receive_timeout(Duration::from_secs(1)).await {
//!         Ok(event) => println!("Received: id={}, message={}", event.id, event.message),
//!         Err(e) => println!("Error: {}", e),
//!     }
//!     
//!     // 9. Shutdown when done
//!     system.shutdown().await?;
//!     
//!     Ok(())
//! }
//! ```
//! 
//! ## Features of the Zero Copy Implementation
//! 
//! - **High Performance**: Benchmarked at ~2.2 million events/sec with 5 subscribers
//! - **Zero Copy**: Events are wrapped in Arc for minimal copying between publishers and subscribers
//! - **Prioritization**: Events can have different priority levels
//! - **Timeout Control**: Set timeouts for receiving operations
//! - **Concurrent Dispatching**: Configure how many events can be dispatched concurrently
//! - **Sharding**: Improve performance by using multiple shards
//! - **Batch Publishing**: Publish multiple events efficiently
//! 
//! ## Configuration Options
//! 
//! When creating a Zero Copy event system, you can configure:
//! 
//! ```rust,no_run
//! use infra_common::events::builder::{EventSystemBuilder, ImplementationType};
//! 
//! let system = EventSystemBuilder::new()
//!     .implementation(ImplementationType::ZeroCopy)
//!     .channel_capacity(10_000)        // Buffer size for channels
//!     .max_concurrent_dispatches(1000)  // Max concurrent event dispatching
//!     .enable_priority(true)           // Enable prioritized event handling
//!     .default_timeout(Some(std::time::Duration::from_secs(1)))  // Default timeout
//!     .shard_count(8)                  // Number of shards for performance
//!     .enable_metrics(true)            // Enable performance metrics
//!     .build();
//! ```

// Shared components
pub mod bus;
pub mod registry;
pub mod subscriber;
pub mod types;
pub mod publisher;

// Core interfaces for the new API
pub mod api;
pub mod static_path;
pub mod zero_copy;
pub mod system;
pub mod builder;

// Tests
#[cfg(test)]
mod tests;

