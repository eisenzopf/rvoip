//! # Call Center Engine for RVOIP
//!
//! This crate provides call center orchestration functionality for the RVOIP stack.
//! It integrates with session-core for SIP handling and provides call center business logic
//! including agent management, call queuing, routing, and monitoring.
//!
//! ## Features
//!
//! - **Call Orchestration**: Central coordination of agent-customer calls
//! - **Agent Management**: Registration, availability tracking, skill-based routing
//! - **Call Queuing**: Priority queues with overflow policies
//! - **Routing Engine**: Business rules and skill-based call distribution
//! - **Monitoring**: Real-time metrics and supervisor features
//! - **Database Integration**: Persistent storage with Limbo database
//!
//! ## Architecture
//!
//! The call center is organized into several key modules:
//!
//! - [`orchestrator`]: Core call center coordination and bridge management
//! - [`agent`]: Agent registration, status tracking, and skill routing
//! - [`queue`]: Call queuing with priorities and overflow handling
//! - [`routing`]: Call routing engine with business rules
//! - [`monitoring`]: Real-time monitoring and analytics
//! - [`api`]: Public APIs for applications
//! - [`integration`]: Session-core integration adapters
//! - [`database`]: Persistent storage with Limbo
//!
//! ## Quick Start
//!
//! ```rust,no_run
//! use rvoip_call_engine::prelude::*;
//! use std::sync::Arc;
//!
//! #[tokio::main]
//! async fn main() -> Result<()> {
//!     // Initialize database
//!     let database = CallCenterDatabase::new_in_memory().await?;
//!     
//!     // Create configuration
//!     let config = CallCenterConfig::default();
//!     
//!     // Initialize call center
//!     let call_center = CallCenterEngine::new(config, database).await?;
//!     
//!     // Start processing calls
//!     call_center.start().await?;
//!     
//!     Ok(())
//! }
//! ```

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

// Database integration
pub mod database;

// Re-exports for convenience
pub use error::{CallCenterError, Result};
pub use config::CallCenterConfig;

// **NEW**: Import the REAL CallCenterEngine with session-core integration
pub use orchestrator::core::CallCenterEngine;

/// Call center statistics
#[derive(Debug, Clone)]
pub struct CallCenterStats {
    pub active_calls: usize,
    pub active_bridges: usize,
    pub total_calls_handled: u64,
}

/// Prelude module for convenient imports
pub mod prelude {
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
    pub use crate::orchestrator::bridge::BridgeType;
    
    // Agent types - import from correct modules
    pub use crate::agent::{
        AgentRegistry, Agent, AgentStatus, SkillBasedRouter, AvailabilityTracker,
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
        CallCenterDatabase,
        agent_store::{Agent as DbAgent, AgentStore, CreateAgentRequest, AgentSkill},
        call_records::{CallRecord, CallDirection, CallStatus as DbCallStatus},
        queue_store::{CallQueue as DbQueue, QueueStore},
        routing_store::{RoutingPolicy, RoutingPolicyType, RoutingStore},
    };
    
    // **NEW**: Session-core integration types
    pub use rvoip_session_core::{
        SessionId, Session,
        api::{ServerSessionManager, ServerConfig, create_full_server_manager},
        session::bridge::{BridgeId, BridgeConfig, BridgeInfo, BridgeEvent}
    };
    pub use rvoip_sip_core::{Request, Response, Method, StatusCode, Uri};
    pub use rvoip_transaction_core::TransactionManager;
    
    // Common external types
    pub use chrono::{DateTime, Utc};
    pub use uuid::Uuid;
} 