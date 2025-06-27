//! Call center orchestration module
//!
//! This module provides the core orchestration functionality for the call center,
//! coordinating between agents, queues, routing, and session-core bridge APIs.

pub mod core;
pub mod types;
pub mod handler;
pub mod routing;
pub mod calls;
pub mod agents;
pub mod bridge_operations;
pub mod bridge;
pub mod lifecycle;

// Export the main call center engine
pub use core::CallCenterEngine;

// Export types
pub use types::{
    CallInfo, AgentInfo, CustomerType, CallStatus, 
    RoutingDecision, RoutingStats, OrchestratorStats
};

// Export handler for advanced use cases
pub use handler::CallCenterCallHandler;

// Export other managers
pub use bridge::BridgeManager;
pub use lifecycle::CallLifecycleManager; 