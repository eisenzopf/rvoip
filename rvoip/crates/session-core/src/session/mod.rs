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
    // ✅ BASIC PRIMITIVES (keeping in session-core)
    SessionDependencyTracker, SessionDependency, DependencyType, DependencyState,
    BasicSessionGroup, BasicGroupType, BasicGroupState, BasicGroupConfig,
    BasicSessionMembership, BasicGroupEvent,
    
    // ⚠️ BUSINESS LOGIC (temporary exports - will be moved to call-engine in Phase 12)
    // Phase 12.1: Group management business logic
    SessionGroupManager, SessionGroup, GroupType, GroupState, GroupConfig,
    // Phase 12.2: Policy management business logic  
    SessionPolicyManager, ResourceSharingPolicy, CoordinationPolicy, ResourceType,
    // Phase 12.3: Priority management business logic
    SessionPriorityManager, SessionPriority, PriorityClass, SchedulingPolicy,
    // Phase 12.4: Event orchestration business logic
    CrossSessionEventPropagator, SessionCoordinationEvent, PropagationRule,
    // Phases 12.2-12.4: Sequence coordination business logic
    SessionSequenceCoordinator, SessionSequence, SequenceType, SequenceState,
}; 