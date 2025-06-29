//! # Call Queue Management Module
//!
//! This module provides comprehensive call queuing functionality for the call center,
//! including priority-based queuing, overflow handling, and advanced queue policies.
//! It's designed to handle high-volume call centers with sophisticated routing requirements.
//!
//! ## Architecture
//!
//! The queue management system follows a multi-layered architecture:
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────────┐
//! │                    Call Center Engine                       │
//! │  (Incoming calls, routing decisions)                       │
//! └─────────────────────────┬───────────────────────────────────┘
//!                           │
//! ┌─────────────────────────▼───────────────────────────────────┐
//! │                   Queue Manager                             │
//! │  - Multi-queue coordination                                 │
//! │  - Priority-based routing                                   │
//! │  - Assignment tracking                                      │
//! │  - Statistics collection                                    │
//! └─────────────────────────┬───────────────────────────────────┘
//!                           │
//! ┌─────────────────────────▼───────────────────────────────────┐
//! │                Individual Queues                            │
//! │  ┌─────────────┐ ┌─────────────┐ ┌─────────────┐           │
//! │  │ Support     │ │    Sales    │ │  Billing    │           │
//! │  │ Queue       │ │    Queue    │ │   Queue     │           │
//! │  │             │ │             │ │             │           │
//! │  │ Priority: 1 │ │ Priority: 2 │ │ Priority: 3 │           │
//! │  └─────────────┘ └─────────────┘ └─────────────┘           │
//! └─────────────────────────┬───────────────────────────────────┘
//!                           │
//! ┌─────────────────────────▼───────────────────────────────────┐
//! │               Policies & Overflow                           │
//! │  - Queue capacity limits                                    │
//! │  - Wait time thresholds                                     │
//! │  - Overflow routing rules                                   │
//! │  - SLA monitoring                                           │
//! └─────────────────────────────────────────────────────────────┘
//! ```
//!
//! ## Core Features
//!
//! ### **Priority-Based Queuing**
//! - Multiple priority levels (0 = highest, 255 = lowest)
//! - Automatic priority insertion and ordering
//! - Priority boosting for waiting calls
//! - VIP customer handling
//!
//! ### **Queue Management**
//! - Multiple named queues with individual policies
//! - Dynamic queue creation and configuration
//! - Capacity limits and overflow handling
//! - Real-time queue statistics and monitoring
//!
//! ### **Assignment Tracking**
//! - Prevents duplicate call processing
//! - Tracks calls being assigned to agents
//! - Automatic cleanup of stuck assignments
//! - Race condition prevention
//!
//! ### **Overflow Handling**
//! - Configurable overflow to alternate queues
//! - Time-based escalation policies
//! - Capacity-based overflow triggers
//! - External service integration
//!
//! ## Quick Start
//!
//! ### Basic Queue Setup
//!
//! ```rust
//! use rvoip_call_engine::queue::{QueueManager, QueuedCall};
//! use rvoip_session_core::SessionId;
//! 
//! # fn example() -> Result<(), Box<dyn std::error::Error>> {
//! // Create queue manager
//! let mut queue_manager = QueueManager::new();
//! 
//! // Create queues for different departments
//! queue_manager.create_queue("support".to_string(), "Technical Support".to_string(), 50)?;
//! queue_manager.create_queue("sales".to_string(), "Sales Team".to_string(), 30)?;
//! queue_manager.create_queue("billing".to_string(), "Billing Department".to_string(), 20)?;
//! 
//! println!("Call queues configured successfully");
//! # Ok(())
//! # }
//! ```
//!
//! ### Enqueueing Calls
//!
//! ```rust
//! use rvoip_call_engine::queue::{QueueManager, QueuedCall};
//! use rvoip_session_core::SessionId;
//! 
//! # fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let mut queue_manager = QueueManager::new();
//! # queue_manager.create_queue("support".to_string(), "Support".to_string(), 50)?;
//! 
//! // Create a high-priority call
//! let urgent_call = QueuedCall {
//!     session_id: SessionId::new(),
//!     caller_id: "+1-555-0123".to_string(),
//!     priority: 1,  // High priority (low number)
//!     queued_at: chrono::Utc::now(),
//!     estimated_wait_time: Some(30), // 30 seconds
//!     retry_count: 0,
//! };
//! 
//! // Queue the call
//! let position = queue_manager.enqueue_call("support", urgent_call)?;
//! println!("Call queued at position: {}", position);
//! 
//! // Create a normal priority call
//! let normal_call = QueuedCall {
//!     session_id: SessionId::new(),
//!     caller_id: "+1-555-0456".to_string(),
//!     priority: 5,  // Normal priority
//!     queued_at: chrono::Utc::now(),
//!     estimated_wait_time: Some(120), // 2 minutes
//!     retry_count: 0,
//! };
//! 
//! queue_manager.enqueue_call("support", normal_call)?;
//! # Ok(())
//! # }
//! ```
//!
//! ### Dequeuing for Agents
//!
//! ```rust
//! use rvoip_call_engine::queue::QueueManager;
//! 
//! # fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let mut queue_manager = QueueManager::new();
//! # queue_manager.create_queue("support".to_string(), "Support".to_string(), 50)?;
//! 
//! // Agent becomes available - get next call
//! if let Some(call) = queue_manager.dequeue_for_agent("support")? {
//!     println!("Assigning call {} to agent", call.session_id);
//!     println!("Caller: {}, Priority: {}", call.caller_id, call.priority);
//!     
//!     // Process call assignment...
//!     // On success: call remains dequeued
//!     // On failure: call can be re-queued
//! } else {
//!     println!("No calls waiting in support queue");
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ### Queue Statistics
//!
//! ```rust
//! use rvoip_call_engine::queue::QueueManager;
//! 
//! # fn example() -> Result<(), Box<dyn std::error::Error>> {
//! # let queue_manager = QueueManager::new();
//! 
//! // Get detailed queue statistics
//! let stats = queue_manager.get_queue_stats("support")?;
//! println!("Queue Statistics:");
//! println!("  Total calls: {}", stats.total_calls);
//! println!("  Average wait: {} seconds", stats.average_wait_time_seconds);
//! println!("  Longest wait: {} seconds", stats.longest_wait_time_seconds);
//! 
//! // Get overall queue metrics
//! let total_calls = queue_manager.total_queued_calls();
//! println!("Total calls across all queues: {}", total_calls);
//! # Ok(())
//! # }
//! ```
//!
//! ## Priority System
//!
//! The queue system uses a numeric priority system where lower numbers indicate higher priority:
//!
//! - **Priority 0**: Emergency/VIP calls (highest)
//! - **Priority 1-2**: High priority calls
//! - **Priority 3-5**: Normal priority calls
//! - **Priority 6-8**: Low priority calls
//! - **Priority 9+**: Lowest priority calls
//!
//! ### Priority Examples
//!
//! ```rust
//! use rvoip_call_engine::queue::QueuedCall;
//! 
//! # fn example() {
//! // VIP customer call
//! let vip_call = QueuedCall {
//!     priority: 0,  // Highest priority
//!     // ... other fields
//! #   session_id: rvoip_session_core::SessionId::new(),
//! #   caller_id: "VIP".to_string(),
//! #   queued_at: chrono::Utc::now(),
//! #   estimated_wait_time: None,
//! #   retry_count: 0,
//! };
//! 
//! // Escalated support call
//! let escalated_call = QueuedCall {
//!     priority: 1,  // High priority
//!     // ... other fields
//! #   session_id: rvoip_session_core::SessionId::new(),
//! #   caller_id: "Escalated".to_string(),
//! #   queued_at: chrono::Utc::now(),
//! #   estimated_wait_time: None,
//! #   retry_count: 1,
//! };
//! 
//! // Normal customer call
//! let normal_call = QueuedCall {
//!     priority: 5,  // Normal priority
//!     // ... other fields
//! #   session_id: rvoip_session_core::SessionId::new(),
//! #   caller_id: "Normal".to_string(),
//! #   queued_at: chrono::Utc::now(),
//! #   estimated_wait_time: None,
//! #   retry_count: 0,
//! };
//! # }
//! ```
//!
//! ## Advanced Features
//!
//! ### **Overflow Management**
//! - Automatic overflow when queues reach capacity
//! - Configurable overflow destinations
//! - Time-based overflow triggers
//! - External service integration for overflow
//!
//! ### **Assignment Protection**
//! - Prevents race conditions during call assignment
//! - Tracks calls being processed
//! - Automatic cleanup of stuck assignments
//! - Retry mechanisms for failed assignments
//!
//! ### **Performance Monitoring**
//! - Real-time queue statistics
//! - SLA compliance tracking
//! - Wait time analysis
//! - Historical performance metrics
//!
//! ## Error Handling
//!
//! The queue system provides comprehensive error handling:
//!
//! ```rust
//! use rvoip_call_engine::queue::QueueManager;
//! use rvoip_call_engine::error::CallCenterError;
//! 
//! # fn example() {
//! let mut queue_manager = QueueManager::new();
//! 
//! match queue_manager.get_queue_stats("nonexistent-queue") {
//!     Ok(stats) => println!("Queue stats: {:?}", stats),
//!     Err(CallCenterError::NotFound(msg)) => println!("Queue not found: {}", msg),
//!     Err(e) => println!("Other error: {}", e),
//! }
//! # }
//! ```
//!
//! ## Production Considerations
//!
//! ### **Scaling**
//! - Monitor queue sizes and adjust capacity limits
//! - Implement queue sharding for very high volumes
//! - Use database persistence for queue durability
//!
//! ### **Performance**
//! - Regular cleanup of expired calls
//! - Monitor assignment tracking overhead
//! - Optimize priority insertion algorithms
//!
//! ### **Reliability**
//! - Implement queue persistence across restarts
//! - Monitor and alert on queue capacity issues
//! - Implement backup overflow destinations
//!
//! ## Modules
//!
//! - [`manager`]: Core queue management and coordination
//! - [`policies`]: Queue policies and configuration
//! - [`overflow`]: Overflow handling and routing

pub mod manager;
pub mod policies;
pub mod overflow;

pub use manager::{CallQueue, QueueManager, QueuedCall, QueueStats};
pub use policies::QueuePolicies;
pub use overflow::OverflowHandler; 