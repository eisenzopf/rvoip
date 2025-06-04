//! Session Coordination Patterns
//! 
//! This module provides advanced coordination patterns for managing relationships
//! between sessions in complex call scenarios:
//! 
//! - Session dependencies and parent-child relationships
//! - Session groups for related sessions (conferences, transfers)
//! - Session sequence coordination (A-leg/B-leg relationships)
//! - Cross-session event propagation and synchronization
//! - Session priority and scheduling management
//! - Resource sharing policies between sessions

// Core coordination types and patterns
pub mod dependencies;
pub mod groups;
pub mod sequences;
pub mod events;
pub mod priority;
pub mod policies;

// Re-export main coordination types
pub use dependencies::{
    SessionDependencyTracker, SessionDependency, DependencyType, DependencyState,
    DependencyConfig, DependencyMetrics
};

pub use groups::{
    SessionGroupManager, SessionGroup, GroupType, GroupState, GroupConfig,
    SessionMembership, GroupEvent, GroupStatistics, GroupManagerConfig, GroupManagerMetrics
};

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