//! Session Coordination Patterns
//! 
//! This module provides basic coordination primitives for session relationships
//! and dependencies. Complex business logic and orchestration is handled by
//! higher layers (call-engine).
//! 
//! ## Scope
//! 
//! **✅ Included (Basic Primitives)**:
//! - Session dependencies and parent-child relationships
//! - Basic session grouping data structures  
//! - Simple session sequence coordination
//! - Basic cross-session event communication (simple pub/sub)
//! - Basic resource tracking and limits
//! - Basic priority classification and QoS levels
//! 
//! **❌ Not Included (Business Logic - moved to call-engine)**:
//! - Advanced group management and orchestration
//! - Complex policy enforcement and resource allocation
//! - Sophisticated priority scheduling and QoS management
//! - Complex event routing and business coordination

// ✅ BASIC PRIMITIVES (keeping in session-core)
pub mod dependencies;
pub mod basic_groups;
pub mod resource_limits;
pub mod basic_priority;
pub mod basic_events;

// ❌ BUSINESS LOGIC MODULES (keeping for backward compatibility during transition)
// These modules contain sophisticated business logic that belongs in call-engine:
// - groups.rs: Conference management, leader election, group policies (934 lines) → call-engine/src/conference/
// - policies.rs: Resource allocation, policy enforcement, business rules (927 lines) → call-engine/src/policy/
// - priority.rs: QoS scheduling, resource allocation, task management (722 lines) → call-engine/src/priority/
// - events.rs: Event orchestration, propagation rules, complex filtering (542 lines) → call-engine/src/orchestrator/
// - sequences.rs: Complex sequence coordination and business orchestration → call-engine/src/sequences/

// NOTE: Business logic modules are temporarily kept for backward compatibility
// but are NOT re-exported. Applications should migrate to call-engine for business logic.


// Re-export ONLY basic coordination primitives
pub use dependencies::{
    SessionDependencyTracker, SessionDependency, DependencyType, DependencyState,
    DependencyConfig, DependencyMetrics
};

pub use basic_groups::{
    BasicSessionGroup, BasicGroupType, BasicGroupState, BasicGroupConfig,
    BasicSessionMembership, BasicGroupEvent
};

pub use resource_limits::{
    BasicResourceType, BasicResourceAllocation, BasicResourceUsage, BasicResourceLimits,
    BasicResourceRequest, BasicResourceStats
};

pub use basic_priority::{
    BasicSessionPriority, BasicPriorityClass, BasicQoSLevel, BasicPriorityInfo,
    BasicPriorityConfig
};

pub use basic_events::{
    BasicSessionEvent, BasicEventBus, BasicEventBusConfig, BasicEventFilter,
    FilteredEventSubscriber
}; 