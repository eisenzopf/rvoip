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
//! - Basic cross-session event communication
//! 
//! **❌ Not Included (Business Logic - moved to call-engine)**:
//! - Advanced group management and orchestration
//! - Complex policy enforcement and resource allocation
//! - Sophisticated priority scheduling and QoS management
//! - Complex event routing and business coordination

// Core coordination primitives
pub mod dependencies;
pub mod basic_groups;
// Note: groups.rs contains business logic - will be moved to call-engine in Phase 12.1
// Note: sequences.rs, events.rs, priority.rs, policies.rs contain business logic - will be refactored in subsequent phases

// Re-export basic coordination primitives
pub use dependencies::{
    SessionDependencyTracker, SessionDependency, DependencyType, DependencyState,
    DependencyConfig, DependencyMetrics
};

pub use basic_groups::{
    BasicSessionGroup, BasicGroupType, BasicGroupState, BasicGroupConfig,
    BasicSessionMembership, BasicGroupEvent
};

// TODO: These will be refactored in subsequent phases to extract basic primitives:
// - sequences.rs → basic sequence primitives only
// - events.rs → basic event bus only  
// - priority.rs, policies.rs → will be moved to call-engine entirely

// Temporary exports during transition (Phase 12.1 focuses on groups first)
pub mod sequences;
pub mod events;
pub mod priority;
pub mod policies;
pub mod groups; // Contains business logic - to be removed after call-engine integration

pub use sequences::{
    SessionSequenceCoordinator, SessionSequence, SequenceType, SequenceState,
    SequenceStep, SequenceStatistics, SequenceConfig, CoordinatorConfig, SequenceMetrics
};

pub use events::{
    CrossSessionEventPropagator, SessionCoordinationEvent, PropagationRule, EventFilter,
    PropagationConfig, PropagationMetrics
};

pub use priority::{
    SessionPriorityManager, SessionPriority, PriorityClass, SchedulingPolicy, SessionPriorityInfo,
    ResourceLimits, ResourceAllocation, QoSLevel, ScheduledTask, PriorityManagerConfig,
    PriorityMetrics, ResourceUsage
};

pub use policies::{
    SessionPolicyManager, ResourceSharingPolicy, CoordinationPolicy, ResourceType,
    EnforcementLevel, PolicyScope, PolicyConfig, ResourceRequest, PolicyViolation,
    ViolationSeverity, PolicyMetrics, PolicyManagerConfig
};

// Business logic exports (to be removed as they move to call-engine)
pub use groups::{
    SessionGroupManager, SessionGroup, GroupType, GroupState, GroupConfig,
    SessionMembership, GroupEvent, GroupStatistics, GroupManagerConfig, GroupManagerMetrics
}; 