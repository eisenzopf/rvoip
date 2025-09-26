//! Common types used throughout the intermediary-core library

use serde::{Deserialize, Serialize};
use std::collections::HashMap;

/// Unique identifier for an intermediary session
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct IntermediarySessionId(pub String);

impl IntermediarySessionId {
    pub fn new() -> Self {
        Self(uuid::Uuid::new_v4().to_string())
    }
}

/// Operating mode for the intermediary
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum IntermediaryMode {
    /// Stateless proxy - forwards without maintaining state
    StatelessProxy,
    /// Stateful proxy - maintains transaction state
    StatefulProxy,
    /// Call-stateful proxy - maintains dialog state
    CallStatefulProxy,
    /// Back-to-back user agent
    B2BUA,
    /// Gateway mode for protocol conversion
    Gateway,
    /// Session Border Controller mode
    SBC,
}

/// Routing decision made by the routing engine
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RoutingDecision {
    /// Target URI(s) for the request
    pub targets: Vec<String>,
    /// Routing method (parallel, sequential, etc.)
    pub method: RoutingMethod,
    /// Any transformations to apply
    pub transformations: Vec<Transformation>,
    /// Policy actions to enforce
    pub policies: Vec<PolicyAction>,
}

/// Method for routing to multiple targets
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub enum RoutingMethod {
    /// Try targets in sequence until one succeeds
    Sequential,
    /// Try all targets in parallel
    Parallel,
    /// Use first available target
    FirstAvailable,
    /// Load balance across targets
    LoadBalance,
}

/// Transformation to apply to a message
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum Transformation {
    /// Modify a header
    ModifyHeader { name: String, value: String },
    /// Remove a header
    RemoveHeader { name: String },
    /// Add a header
    AddHeader { name: String, value: String },
    /// Modify the request URI
    ModifyRequestUri { uri: String },
    /// Strip or add prefixes
    NumberTranslation { pattern: String, replacement: String },
}

/// Policy action to enforce
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PolicyAction {
    /// Allow the request to proceed
    Allow,
    /// Reject the request with a reason
    Reject { code: u16, reason: String },
    /// Redirect to another destination
    Redirect { target: String },
    /// Rate limit the sender
    RateLimit { max_per_second: u32 },
    /// Require authentication
    RequireAuth,
    /// Log for audit purposes
    AuditLog,
}