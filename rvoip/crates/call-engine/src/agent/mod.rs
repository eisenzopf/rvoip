//! Agent management module
//!
//! This module provides agent registration, availability tracking,
//! and skill-based routing functionality for the call center.

pub mod registry;
pub mod routing;
pub mod availability;

pub use registry::{AgentRegistry, Agent, AgentStatus};
pub use routing::SkillBasedRouter;
pub use availability::AvailabilityTracker; 