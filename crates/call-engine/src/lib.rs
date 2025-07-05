//! # RVOIP Call Engine
//!
//! A comprehensive call center orchestration engine built on top of the RVOIP SIP stack.
//! This crate provides enterprise-grade call center functionality including agent management,
//! call queuing, intelligent routing, and real-time monitoring.
//!
//! ## Overview
//!
//! The Call Engine is the heart of a modern call center system, providing:
//!
//! - **Call Orchestration**: Central coordination of agent-customer calls with SIP bridge management
//! - **Agent Management**: Registration, availability tracking, skill-based routing, and performance monitoring
//! - **Call Queuing**: Priority-based queues with overflow policies and wait time management
//! - **Intelligent Routing**: Business rules engine with skill matching and load balancing
//! - **Real-time Monitoring**: Live dashboards, quality metrics, and supervisor tools
//! - **Database Integration**: Persistent storage with SQLite (Limbo) for scalability
//!
//! ## Architecture
//!
//! The call center is built on a modular architecture:
//!
//! ```text
//! ┌─────────────────┐    ┌─────────────────┐    ┌─────────────────┐
//! │   Client API    │    │  Supervisor API │    │    Admin API    │
//! └─────────────────┘    └─────────────────┘    └─────────────────┘
//!           │                       │                       │
//!           └───────────────────────┼───────────────────────┘
//!                                   │
//!                      ┌─────────────────┐
//!                      │ CallCenterEngine │
//!                      └─────────────────┘
//!                                   │
//!           ┌───────────────────────┼───────────────────────┐
//!           │                       │                       │
//!  ┌─────────────────┐    ┌─────────────────┐    ┌─────────────────┐
//!  │  Agent Registry │    │  Queue Manager  │    │ Routing Engine  │
//!  └─────────────────┘    └─────────────────┘    └─────────────────┘
//!           │                       │                       │
//!           └───────────────────────┼───────────────────────┘
//!                                   │
//!                        ┌─────────────────┐
//!                        │ Session Manager │ (rvoip-session-core)
//!                        └─────────────────┘
//! ```
//!
//! ## Quick Start
//!
//! ### Basic Call Center Setup
//!
//! ```
//! use rvoip_call_engine::prelude::*;
//! 
//! # async fn example() -> Result<()> {
//! // Create configuration with sensible defaults
//! let config = CallCenterConfig::default();
//! 
//! // Create call center with in-memory database for this example
//! let call_center = CallCenterEngine::new(config, None).await?;
//! 
//! println!("Call center engine created successfully!");
//! # Ok(())
//! # }
//! ```
//!
//! ### Agent Registration
//!
//! ```
//! use rvoip_call_engine::prelude::*;
//! 
//! # async fn example() -> Result<()> {
//! # let call_center = CallCenterEngine::new(CallCenterConfig::default(), None).await?;
//! // Define an agent with skills and capabilities
//! let agent = Agent {
//!     id: "agent-001".to_string(),
//!     sip_uri: "sip:alice@call-center.local".to_string(),
//!     display_name: "Alice Johnson".to_string(),
//!     skills: vec!["english".to_string(), "sales".to_string(), "tier1".to_string()],
//!     max_concurrent_calls: 2,
//!     status: AgentStatus::Available,
//!     department: Some("sales".to_string()),
//!     extension: Some("1001".to_string()),
//! };
//! 
//! // Register the agent (this creates a SIP session)
//! let session_id = call_center.register_agent(&agent).await?;
//! println!("Agent {} registered with session ID: {}", agent.display_name, session_id);
//! # Ok(())
//! # }
//! ```
//!
//! ### Advanced Routing Configuration
//!
//! ```
//! use rvoip_call_engine::prelude::*;
//! 
//! # async fn example() -> Result<()> {
//! let mut config = CallCenterConfig::default();
//! 
//! // Configure skill-based routing
//! config.routing.default_strategy = RoutingStrategy::SkillBased;
//! config.routing.enable_load_balancing = true;
//! config.routing.load_balance_strategy = LoadBalanceStrategy::LeastBusy;
//! 
//! // Configure queue settings
//! config.queues.default_max_wait_time = 300; // 5 minutes max wait
//! config.queues.max_queue_size = 50;
//! config.queues.enable_priorities = true;
//! 
//! // Configure agent settings
//! config.agents.enable_skill_based_routing = true;
//! config.agents.default_skills = vec!["general".to_string(), "english".to_string()];
//! 
//! let call_center = CallCenterEngine::new(config, None).await?;
//! # Ok(())
//! # }
//! ```
//!
//! ### Monitoring and Statistics
//!
//! ```
//! use rvoip_call_engine::prelude::*;
//! 
//! # async fn example() -> Result<()> {
//! # let call_center = CallCenterEngine::new(CallCenterConfig::default(), None).await?;
//! // Get real-time statistics
//! let stats = call_center.get_stats().await;
//! println!("Call Center Status:");
//! println!("  Active calls: {}", stats.active_calls);
//! println!("  Available agents: {}", stats.available_agents);
//! println!("  Queued calls: {}", stats.queued_calls);
//! println!("  Total calls handled: {}", stats.total_calls_handled);
//! 
//! // Get detailed routing statistics
//! println!("Routing Performance:");
//! println!("  Direct routes: {}", stats.routing_stats.calls_routed_directly);
//! println!("  Queued calls: {}", stats.routing_stats.calls_queued);
//! println!("  Rejected calls: {}", stats.routing_stats.calls_rejected);
//! # Ok(())
//! # }
//! ```
//!
//! ## Key Modules
//!
//! - [`orchestrator`]: Core call center coordination and bridge management
//! - [`agent`]: Agent registration, status tracking, and skill-based routing
//! - [`queue`]: Call queuing with priorities and overflow handling
//! - [`routing`]: Intelligent call routing engine with business rules
//! - [`monitoring`]: Real-time monitoring, metrics collection, and analytics
//! - [`api`]: Public APIs for client applications, supervisors, and administrators
//! - [`integration`]: Session-core integration adapters and handlers
//! - [`database`]: Persistent storage with Limbo SQLite database
//! - [`config`]: Configuration management and validation
//! - [`error`]: Comprehensive error handling and result types
//!
//! ## Integration with RVOIP Stack
//!
//! This crate integrates seamlessly with other RVOIP components:
//!
//! - **rvoip-session-core**: SIP session management and call handling
//! - **rvoip-sip-core**: Low-level SIP protocol handling
//! - **rvoip-media-core**: Audio processing and codec management
//! - **rvoip-client-core**: Client-side integration for softphones
//!
//! ## Network Configuration
//!
//! The call engine properly respects and propagates configured bind addresses:
//!
//! ```
//! use rvoip_call_engine::prelude::*;
//! 
//! # async fn example() -> Result<()> {
//! // Configure specific IP addresses for your deployment
//! let mut config = CallCenterConfig::default();
//! config.general.local_signaling_addr = "127.0.0.1:5060".parse().unwrap();  // Use localhost for test
//! config.general.local_media_addr = "127.0.0.1:20000".parse().unwrap();     // Same IP for media
//! 
//! // The engine ensures these addresses propagate to all layers
//! let engine = CallCenterEngine::new(config, None).await?;
//! # Ok(())
//! # }
//! ```
//!
//! Key points:
//! - Configured IPs propagate through session-core to dialog-core and transport
//! - No more hardcoded 0.0.0.0 addresses - your specific IP is used everywhere  
//! - Media port range starts from the configured `local_media_addr` port
//!
//! For automatic media port allocation, the engine uses the port from `local_media_addr`
//! as the base and allocates a range from there (typically +1000 ports).
//!
//! ## Production Deployment
//!
//! For production deployments, consider:
//!
//! - **Database**: Use a dedicated database file with regular backups
//! - **Monitoring**: Enable real-time monitoring and quality alerts
//! - **Security**: Configure proper authentication and authorization
//! - **Scaling**: Monitor agent and call limits, scale horizontally as needed
//! - **Network**: Ensure proper SIP and RTP port configuration with specific bind addresses
//!
//! ## Examples
//!
//! See the `examples/` directory for complete working examples:
//!
//! - `e2e_test/`: End-to-end call center testing setup
//! - Basic call center server implementation
//! - Agent client simulation
//! - Load testing scenarios

// Core modules
pub mod error;
pub mod config;

// Call center functionality modules
pub mod orchestrator;
pub mod agent;
pub mod queue;
pub mod routing;
pub mod monitoring;

// External interfaces
pub mod api;
pub mod integration;
pub mod server;

// Database integration
pub mod database;

// Re-exports for convenience
pub use error::{CallCenterError, Result};
pub use config::CallCenterConfig;

// **NEW**: Import the REAL CallCenterEngine with session-core integration
pub use orchestrator::core::CallCenterEngine;

// Export API types
pub use api::{CallCenterClient, SupervisorApi, AdminApi};

/// Call center statistics and performance metrics
///
/// This struct provides a snapshot of the call center's current operational state,
/// including active calls, agent availability, and routing performance.
#[derive(Debug, Clone)]
pub struct CallCenterStats {
    /// Number of currently active calls in the system
    pub active_calls: usize,
    /// Number of active SIP bridges connecting agents to customers
    pub active_bridges: usize,
    /// Total number of calls handled since startup
    pub total_calls_handled: u64,
}

/// Prelude module for convenient imports
///
/// Import this module to get access to the most commonly used types and traits:
///
/// ```
/// use rvoip_call_engine::prelude::*;
/// ```
pub mod prelude {
    //! Commonly used types and traits for call center applications
    //!
    //! This module re-exports the most frequently used items from the call engine,
    //! making it easy to get started with a single import.
    
    // **UPDATED**: Core types - now using REAL CallCenterEngine
    pub use crate::{CallCenterError, CallCenterConfig, Result, CallCenterStats};
    
    // **NEW**: Real CallCenterEngine with session-core integration
    pub use crate::orchestrator::core::CallCenterEngine;
    
    // Configuration types
    pub use crate::config::{
        GeneralConfig, AgentConfig, QueueConfig, RoutingConfig, MonitoringConfig, DatabaseConfig,
        RoutingStrategy, LoadBalanceStrategy,
    };
    
    // Orchestrator types - import from correct modules
    pub use crate::orchestrator::{
        BridgeManager, CallLifecycleManager,
        CallInfo, CallStatus, RoutingDecision, OrchestratorStats,
    };
    pub use crate::orchestrator::bridge::{BridgeType, CallCenterBridgeConfig, BridgeStats};
    
    // Agent types - import from correct modules
    pub use crate::agent::{
        AgentRegistry, Agent, AgentId, AgentStatus, SkillBasedRouter, AvailabilityTracker,
    };
    pub use crate::agent::registry::AgentStats;
    
    // Queue types - import from correct modules
    pub use crate::queue::{
        QueueManager, CallQueue, QueuePolicies, OverflowHandler,
    };
    pub use crate::queue::manager::{QueuedCall, QueueStats};
    
    // Routing types
    pub use crate::routing::{
        RoutingEngine, RoutingPolicies, SkillMatcher,
    };
    
    // Monitoring types
    pub use crate::monitoring::{
        SupervisorMonitor, MetricsCollector, CallCenterEvents,
    };
    
    // Database types
    pub use crate::database::{
        DatabaseManager,
        DbAgent,
        DbAgentStatus,
        DbQueuedCall,
        DbActiveCall,
    };
    
    // Session-core integration types
    pub use rvoip_session_core::{
        // Basic session types
        SessionId, CallSession, CallState,
        // Session management
        SessionCoordinator, SessionManagerBuilder,
        // Call handling
        CallHandler, IncomingCall, CallDecision,
        // Bridge management  
        BridgeId, BridgeInfo, BridgeEvent,
    };
    // StatusCode is available from session-core's types module
    pub use rvoip_session_core::types::StatusCode;
    
    // Note: Uri, Request, Response, Method are no longer directly accessible
    // Use session-core's high-level APIs instead
    
    // Common external types
    pub use chrono::{DateTime, Utc};
    pub use uuid::Uuid;
} 

pub use server::{CallCenterServer, CallCenterServerBuilder}; 