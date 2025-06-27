//! Agent management module for the call center
//!
//! This module provides functionality for managing call center agents,
//! including registration, skill tracking, availability, and call routing.

pub mod registry;
pub mod routing;
pub mod availability;
pub mod registration;

use std::fmt;
use serde::{Deserialize, Serialize};

/// Agent identifier type
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct AgentId(pub String);

impl From<String> for AgentId {
    fn from(s: String) -> Self {
        AgentId(s)
    }
}

impl From<&str> for AgentId {
    fn from(s: &str) -> Self {
        AgentId(s.to_string())
    }
}

impl fmt::Display for AgentId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl AsRef<str> for AgentId {
    fn as_ref(&self) -> &str {
        &self.0
    }
}

pub use registry::{AgentRegistry, Agent, AgentStatus, AgentStats};
pub use routing::SkillBasedRouter;
pub use availability::AvailabilityTracker;
pub use registration::{SipRegistrar, Registration, RegistrationResponse, RegistrationStatus}; 