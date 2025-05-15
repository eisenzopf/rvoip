// Session module - Handles SIP sessions and call management
mod session_id;
mod session_types;
mod session_config;
// Add the missing modules
pub mod session;
pub mod manager;

// Re-export main types
pub use session_id::SessionId;
pub use session_types::{SessionState, SessionDirection, SessionTransactionType};
pub use session_config::SessionConfig;
pub use session::Session;
pub use manager::SessionManager; 