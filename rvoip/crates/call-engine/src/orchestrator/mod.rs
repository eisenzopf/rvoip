//! Call center orchestration module
//!
//! This module provides the core orchestration functionality for the call center,
//! coordinating between agents, queues, routing, and session-core bridge APIs.

pub mod core;
pub mod bridge;
pub mod lifecycle;

// Export the main call center engine with real session-core integration
pub use core::{CallCenterEngine, CallInfo, CallStatus, RoutingDecision, OrchestratorStats};
pub use bridge::BridgeManager;
pub use lifecycle::CallLifecycleManager; 