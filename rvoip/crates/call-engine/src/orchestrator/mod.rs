//! Call center orchestration module
//!
//! This module provides the core orchestration functionality for the call center,
//! coordinating between agents, queues, routing, and session-core bridge APIs.

pub mod core;
pub mod bridge;
pub mod lifecycle;

pub use core::CallOrchestrator;
pub use bridge::BridgeManager;
pub use lifecycle::CallLifecycleManager; 