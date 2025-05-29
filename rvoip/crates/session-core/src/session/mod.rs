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