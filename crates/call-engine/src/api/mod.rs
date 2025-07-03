//! Public API module for call center applications
//!
//! This module provides three distinct API interfaces for different types of users
//! and use cases within the call center ecosystem. Each API is designed with
//! appropriate permissions and functionality for its target audience.
//!
//! # API Overview
//!
//! ## Client API ([`CallCenterClient`])
//!
//! The Client API is designed for **agent applications** and **softphones** that need to:
//! - Register agents with the call center
//! - Handle incoming calls
//! - Manage call states (hold, transfer, hangup)
//! - Update agent status and availability
//! - Access basic call statistics
//!
//! **Target Users**: Agent software, softphones, agent dashboards
//!
//! ## Supervisor API ([`SupervisorApi`])
//!
//! The Supervisor API provides **real-time monitoring** and **limited control** capabilities for:
//! - Monitoring active calls and agent status
//! - Viewing queue statistics and wait times
//! - Manually assigning calls to specific agents
//! - Accessing detailed performance metrics
//! - Monitoring call quality and generating alerts
//!
//! **Target Users**: Supervisor dashboards, quality monitoring systems, real-time analytics
//!
//! ## Admin API ([`AdminApi`])
//!
//! The Admin API offers **full administrative control** for:
//! - System configuration management
//! - Agent management (create, update, delete)
//! - Queue configuration and management
//! - Routing rules and policies
//! - System maintenance and troubleshooting
//! - Historical reporting and analytics
//!
//! **Target Users**: Call center administrators, system integrators, management tools
//!
//! # Permission Model
//!
//! ```text
//! ┌─────────────────────────────────────────────────────────┐
//! │                    Admin API                            │
//! │  • Full system control                                  │
//! │  • Configuration management                             │
//! │  • Agent lifecycle management                           │
//! │  • Historical reporting                                 │
//! ├─────────────────────────────────────────────────────────┤
//! │                  Supervisor API                         │
//! │  • Real-time monitoring                                 │
//! │  • Call assignment control                              │
//! │  • Quality monitoring                                   │
//! │  • Performance metrics                                  │
//! ├─────────────────────────────────────────────────────────┤
//! │                   Client API                            │
//! │  • Agent registration                                   │
//! │  • Call handling                                        │
//! │  • Status management                                    │
//! │  • Basic statistics                                     │
//! └─────────────────────────────────────────────────────────┘
//! ```
//!
//! # Examples
//!
//! ## Agent Application using Client API
//!
//! ```
//! use rvoip_call_engine::prelude::*;
//! 
//! # async fn example() -> Result<()> {
//! # let engine = CallCenterEngine::new(CallCenterConfig::default(), None).await?;
//! 
//! // Create agent structure for registration
//! let agent = Agent {
//!     id: "agent-001".to_string(),
//!     sip_uri: "sip:alice@call-center.local".to_string(),
//!     display_name: "Alice Johnson".to_string(),
//!     skills: vec!["english".to_string(), "sales".to_string()],
//!     max_concurrent_calls: 2,
//!     status: AgentStatus::Available,
//!     department: Some("sales".to_string()),
//!     extension: Some("1001".to_string()),
//! };
//! 
//! println!("Agent configured: {}", agent.display_name);
//! # Ok(())
//! # }
//! ```
//!
//! ## Supervisor Dashboard
//!
//! ```
//! use rvoip_call_engine::prelude::*;
//! 
//! # async fn example() -> Result<()> {
//! # let engine = CallCenterEngine::new(CallCenterConfig::default(), None).await?;
//! 
//! // Get real-time statistics from engine
//! let stats = engine.get_stats().await;
//! println!("Active calls: {}", stats.active_calls);
//! println!("Available agents: {}", stats.available_agents);
//! println!("Queue length: {}", stats.queued_calls);
//! 
//! # Ok(())
//! # }
//! ```
//!
//! ## System Administration
//!
//! ```
//! use rvoip_call_engine::prelude::*;
//! 
//! # async fn example() -> Result<()> {
//! # let engine = CallCenterEngine::new(CallCenterConfig::default(), None).await?;
//! 
//! // Configure a new queue configuration
//! let queue_config = QueueConfig {
//!     default_max_wait_time: 300, // 5 minutes
//!     max_queue_size: 50,
//!     enable_priorities: true,
//!     enable_overflow: true,
//!     announcement_interval: 30,
//! };
//! 
//! // Configure routing
//! let mut routing_config = RoutingConfig::default();
//! routing_config.default_strategy = RoutingStrategy::SkillBased;
//! routing_config.enable_load_balancing = true;
//! 
//! println!("Configuration prepared for deployment");
//! # Ok(())
//! # }
//! ```
//!
//! # API Integration Patterns
//!
//! ## Configuration Example
//!
//! ```
//! use rvoip_call_engine::prelude::*;
//! 
//! # async fn example() -> Result<()> {
//! # let engine = CallCenterEngine::new(CallCenterConfig::default(), None).await?;
//! 
//! // Example configuration and monitoring setup
//! let config = engine.config();
//! println!("Max concurrent calls: {}", config.general.max_concurrent_calls);
//! 
//! # Ok(())
//! # }
//! ```
//!
//! ## REST API Integration
//!
//! ```
//! use rvoip_call_engine::prelude::*;
//! 
//! # async fn example() {
//! # let engine = CallCenterEngine::new(CallCenterConfig::default(), None).await.unwrap();
//! // Example of getting stats for REST API
//! let stats = engine.get_stats().await;
//! println!("Stats for API response: {} active calls", stats.active_calls);
//! # }
//! ```
//!
//! # Security Considerations
//!
//! - **Authentication**: Implement proper authentication for each API level
//! - **Authorization**: Enforce permission boundaries between API types
//! - **Rate Limiting**: Protect against API abuse with rate limiting
//! - **Audit Logging**: Log all administrative actions for compliance
//! - **Encryption**: Use TLS for API communications in production
//!
//! # Error Handling
//!
//! All APIs use the common [`CallCenterError`] type for consistent error handling:
//!
//! ```
//! use rvoip_call_engine::prelude::*;
//! 
//! # async fn example() {
//! // Example error handling pattern
//! match CallCenterError::not_found("agent-001") {
//!     err => println!("Error example: {}", err),
//! }
//! # }
//! ```

pub mod client;
pub mod supervisor;
pub mod admin;

pub use client::CallCenterClient;
pub use supervisor::SupervisorApi;
pub use admin::AdminApi; 