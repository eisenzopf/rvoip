// Core modules
pub mod api;
pub mod state_table;
pub mod state_machine;
pub mod session_store;
pub mod adapters;
pub mod errors;

// Re-export main types from API
pub use api::{
    UnifiedSession, UnifiedCoordinator, SessionEvent, SessionBuilder,
    SessionId, CallState, Role, EventType,
    Result, SessionError,
};

// Re-export internal types for advanced usage
pub use session_store::{SessionStore, SessionState};
pub use state_machine::StateMachine;

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