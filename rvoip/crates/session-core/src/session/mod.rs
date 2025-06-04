// Session module - Handles SIP sessions and call management
// Refactored into focused, modular components for better maintainability

mod session_id;
mod session_config;
pub mod session_types;

// Session implementation - now modular
pub mod session;

// Manager implementation - now modular  
pub mod manager;

// **NEW**: Call lifecycle coordination for session layer (moved from dialog layer)
pub mod call_lifecycle;

// **NEW**: Multi-session bridge infrastructure for call-engine
pub mod bridge;

// **NEW**: Session resource management
pub mod resource;

// **NEW**: Session debugging and tracing utilities
pub mod debug;

// **NEW**: Session coordination patterns for multi-session management
pub mod coordination;

// Re-export main types
pub use session_id::SessionId;
pub use session_types::{
    SessionState, SessionDirection, SessionTransactionType,
    TransferId, TransferState, TransferType, TransferContext
};
pub use session_config::SessionConfig;
pub use session::{Session, SessionMediaState};
pub use manager::SessionManager;
pub use call_lifecycle::CallLifecycleCoordinator;

// **NEW**: Re-export bridge types for call-engine API
pub use bridge::{
    SessionBridge, BridgeId, BridgeState, BridgeInfo, BridgeConfig,
    BridgeEvent, BridgeEventType, BridgeStats, BridgeError
};

// **NEW**: Export resource types
pub use resource::{SessionResourceManager, SessionResourceMetrics, SessionResourceConfig, UserSessionLimits};

// **NEW**: Export debug types
pub use debug::{
    SessionTracer, SessionCorrelationId, SessionLifecycleEvent, SessionLifecycleEventType,
    SessionDebugInfo, SessionStatistics, SessionHealthStatus, SessionDebugger
};

// **NEW**: Export coordination types
pub use coordination::{
    SessionDependencyTracker, SessionDependency, DependencyType, DependencyState,
    SessionGroupManager, SessionGroup, GroupType, GroupState, GroupConfig,
    SessionSequenceCoordinator, SessionSequence, SequenceType, SequenceState,
    CrossSessionEventPropagator, SessionCoordinationEvent, PropagationRule,
    SessionPriorityManager, SessionPriority, PriorityClass, SchedulingPolicy,
    SessionPolicyManager, ResourceSharingPolicy, CoordinationPolicy, ResourceType
}; 