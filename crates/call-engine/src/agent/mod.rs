//! Agent management module for the call center
//!
//! This module provides comprehensive functionality for managing call center agents,
//! including registration, skill tracking, availability monitoring, and intelligent
//! routing based on agent capabilities and workload.
//!
//! # Core Concepts
//!
//! ## Agent Lifecycle
//!
//! 1. **Registration**: Agents register with the call center using SIP REGISTER
//! 2. **Authentication**: Agent credentials are validated
//! 3. **Availability Tracking**: Agent status is monitored (Available, Busy, Away, etc.)
//! 4. **Skill Matching**: Incoming calls are matched with agents based on required skills
//! 5. **Load Balancing**: Call distribution considers agent workload and availability
//! 6. **Logout**: Agents can log out or be automatically logged out after timeout
//!
//! ## Agent Skills
//!
//! Skills are tags that define agent capabilities:
//! - **Language Skills**: "english", "spanish", "french"
//! - **Department Skills**: "sales", "support", "billing"
//! - **Tier Skills**: "tier1", "tier2", "tier3"
//! - **Product Skills**: "product-a", "product-b"
//! - **Special Skills**: "manager", "escalation", "vip"
//!
//! ## Agent Status
//!
//! - [`AgentStatus::Available`]: Ready to take calls
//! - [`AgentStatus::Busy`]: Currently on a call
//! - [`AgentStatus::Away`]: Temporarily unavailable
//! - [`AgentStatus::PostCallWork`]: Completing call documentation
//! - [`AgentStatus::Break`]: On scheduled break
//! - [`AgentStatus::Offline`]: Logged out or disconnected
//!
//! # Examples
//!
//! ## Basic Agent Registration
//!
//! ```
//! use rvoip_call_engine::prelude::*;
//! 
//! # async fn example() -> Result<()> {
//! let mut registry = AgentRegistry::new();
//! 
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
//! registry.register_agent(agent).await?;
//! # Ok(())
//! # }
//! ```
//!
//! ## Skill-Based Routing
//!
//! ```
//! use rvoip_call_engine::prelude::*;
//! 
//! # async fn example() -> Result<()> {
//! let router = SkillBasedRouter::new();
//! let mut registry = AgentRegistry::new();
//! 
//! // Register agents with different skills
//! let sales_agent = Agent {
//!     id: "sales-001".to_string(),
//!     skills: vec!["english".to_string(), "sales".to_string()],
//!     status: AgentStatus::Available,
//!     sip_uri: "sip:sales001@example.com".to_string(),
//!     display_name: "Sales Agent".to_string(),
//!     max_concurrent_calls: 2,
//!     department: None,
//!     extension: None,
//! };
//! 
//! let support_agent = Agent {
//!     id: "support-001".to_string(),
//!     skills: vec!["english".to_string(), "support".to_string(), "tier2".to_string()],
//!     status: AgentStatus::Available,
//!     sip_uri: "sip:support001@example.com".to_string(),
//!     display_name: "Support Agent".to_string(),
//!     max_concurrent_calls: 3,
//!     department: None,
//!     extension: None,
//! };
//! 
//! registry.register_agent(sales_agent).await?;
//! registry.register_agent(support_agent).await?;
//! 
//! // Route call requiring sales skills
//! let required_skills = vec!["sales".to_string()];
//! if let Ok(Some(agent_id)) = router.find_best_agent(&required_skills).await {
//!     println!("Routing to sales agent: {}", agent_id);
//! }
//! # Ok(())
//! # }
//! ```
//!
//! ## Agent Availability Tracking
//!
//! ```
//! use rvoip_call_engine::prelude::*;
//! 
//! # async fn example() -> Result<()> {
//! let tracker = AvailabilityTracker::new();
//! let agent_id = "agent-001";
//! 
//! // Check if agent is active (simple availability check)
//! if tracker.is_agent_active(agent_id, 300) {
//!     println!("Agent is active and ready for calls");
//! } else {
//!     println!("Agent is not active or timed out");
//! }
//! # Ok(())
//! # }
//! ```

pub mod registry;
pub mod routing;
pub mod availability;
pub mod registration;

use std::fmt;
use serde::{Deserialize, Serialize};

/// Agent identifier type for strongly-typed agent references
///
/// This wrapper type provides type safety for agent identifiers throughout
/// the call center system, preventing confusion between agent IDs and other
/// string identifiers.
///
/// # Examples
///
/// ```
/// use rvoip_call_engine::agent::AgentId;
/// 
/// let agent_id = AgentId::from("agent-001");
/// println!("Agent ID: {}", agent_id);
/// 
/// // Convert from string references
/// let agent_id2 = AgentId::from("agent-002");
/// assert_ne!(agent_id, agent_id2);
/// ```
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