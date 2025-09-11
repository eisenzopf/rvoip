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
//! use rvoip_infra_common::events::system::EventSystem;
//! use rvoip_infra_common::events::builder::{EventSystemBuilder, ImplementationType};
//! use rvoip_infra_common::events::types::{Event, EventPriority};
//! use rvoip_infra_common::events::api::EventSystem as EventSystemTrait;
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
//! - **Event Filtering**: Filter events based on their content
//! 
//! ## Configuration Options
//! 
//! When creating a Zero Copy event system, you can configure:
//! 
//! ```rust,no_run
//! use rvoip_infra_common::events::builder::{EventSystemBuilder, ImplementationType};
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
//!
//! ## Event Filtering
//!
//! The event system supports filtering events based on their content. This allows subscribers
//! to only receive events that meet specific criteria:
//!
//! ```rust,no_run
//! use rvoip_infra_common::events::system::EventSystem;
//! use rvoip_infra_common::events::builder::{EventSystemBuilder, ImplementationType};
//! use rvoip_infra_common::events::types::{Event, EventPriority, EventFilter};
//! use rvoip_infra_common::events::api::{EventSystem as EventSystemTrait, EventSubscriber, filters};
//! use std::any::Any;
//! use std::sync::Arc;
//! use serde::{Serialize, Deserialize};
//!
//! // Define an event type
//! #[derive(Clone, Debug, Serialize, Deserialize)]
//! struct UserEvent {
//!     user_id: u32,
//!     action: String,
//!     level: u8,
//! }
//!
//! // Implement the Event trait
//! impl Event for UserEvent {
//!     fn event_type() -> &'static str { "user_event" }
//!     fn priority() -> EventPriority { EventPriority::Normal }
//!     fn as_any(&self) -> &dyn Any { self }
//! }
//!
//! async fn filtering_example() -> Result<(), Box<dyn std::error::Error>> {
//!     let system = EventSystemBuilder::new()
//!         .implementation(ImplementationType::ZeroCopy)
//!         .build();
//!
//!     system.start().await?;
//!
//!     // Method 1: Subscribe with filter function
//!     let mut admin_events = system.subscribe_filtered::<UserEvent, _>(|event| {
//!         event.user_id == 1 && event.level >= 5  // Only admin (ID 1) and high-level events
//!     }).await?;
//!
//!     // Method 2: Using the filter utilities directly
//!     let user_filter = filters::field_equals(|e: &UserEvent| &e.user_id, 42);
//!     let level_filter = filters::field_matches(|e: &UserEvent| &e.level, |level| *level > 3);
//!     
//!     // Combine filters with AND
//!     let combined_filter = filters::and(user_filter, level_filter);
//!     
//!     let mut filtered_events = system.subscribe_with_filter::<UserEvent>(combined_filter).await?;
//!
//!     // Publishing events
//!     let publisher = system.create_publisher::<UserEvent>();
//!     
//!     // This will be received by the admin_events subscriber
//!     publisher.publish(UserEvent {
//!         user_id: 1,
//!         action: "login".to_string(),
//!         level: 8,
//!     }).await?;
//!     
//!     Ok(())
//! }
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

// Phase 2.5: Global Event Coordination for Monolithic Event Integration
pub mod coordinator;
pub mod cross_crate;
pub mod config;
pub mod transport;

// Re-export commonly used items
pub use coordinator::{global_coordinator, GlobalEventCoordinator};
pub use config::{EventCoordinatorConfig, DeploymentConfig};

