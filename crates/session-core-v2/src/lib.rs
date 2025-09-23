// Core modules
pub mod api;
pub mod state_table;
pub mod state_machine;
pub mod session_store;
pub mod adapters;
pub mod errors;

// New core infrastructure
pub mod session_registry;
pub mod types;


// Re-export main types from API
pub use api::{
    UnifiedCoordinator, SessionBuilder,
    SimplePeer, CallId,
};

// Re-export from state_table for correct types
pub use state_table::types::{
    SessionId, Role, EventType,
};

// Re-export CallState from types
pub use types::CallState;

// Re-export error types
pub use errors::{Result, SessionError};

// Re-export internal types for advanced usage
pub use session_store::{
    SessionStore, SessionState, NegotiatedConfig,
    SessionHistory, HistoryConfig, TransitionRecord, GuardResult, ActionRecord,
    SessionInspection, PossibleTransition, SessionHealth, ResourceUsage,
    CleanupConfig, CleanupStats, ResourceLimits,
};
pub use state_machine::StateMachine;
pub use state_table::{Guard, Action};
pub use adapters::{DialogAdapter, MediaAdapter};

/// Session-core v2 with state table architecture
/// 
/// This is a refactored version of session-core that uses a master state table
/// to coordinate between dialog-core and media-core. The key benefits are:
/// 
/// 1. Deterministic state transitions
/// 2. Simplified event handling
/// 3. Easier testing and verification
/// 4. Reduced complexity
/// 
/// The architecture consists of:
/// - State Table: Defines all valid transitions
/// - State Machine: Executes transitions
/// - Session Store: Maintains session state
/// - Coordinator: Routes events to state machine
/// - Adapters: Interface with dialog-core and media-core
#[cfg(test)]
mod tests {
    use super::*;
    
    #[test]
    fn test_state_table_loads() {
        // This will panic if the state table is invalid
        let _ = &*state_table::MASTER_TABLE;
    }
}