//! Agent management module
//!
//! This module provides agent registration, availability tracking,
//! and skill-based routing functionality for the call center.

pub mod registry;
pub mod routing;
pub mod availability;
pub mod registration;

/// Agent identifier type alias
pub type AgentId = String;

pub use registry::{AgentRegistry, Agent, AgentStatus};
pub use routing::SkillBasedRouter;
pub use availability::AvailabilityTracker;
pub use registration::{SipRegistrar, Registration, RegistrationResponse, RegistrationStatus}; 