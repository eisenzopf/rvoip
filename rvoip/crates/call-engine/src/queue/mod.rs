//! Call queue management module
//!
//! This module provides call queuing, priority handling, and overflow routing
//! functionality for the call center.

pub mod manager;
pub mod policies;
pub mod overflow;

pub use manager::{CallQueue, QueueManager, QueuedCall, QueueStats};
pub use policies::QueuePolicies;
pub use overflow::OverflowHandler; 