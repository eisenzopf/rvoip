// RVOIP Session Core Library
//
// This library provides Session Initiation Protocol (SIP) session and dialog management 
// for the RVOIP stack. It serves as the middle layer between low-level SIP transaction 
// processing and high-level application logic.

pub mod api;
pub mod session;
pub mod manager;
pub mod coordination;
pub mod bridge;
pub mod events;

// Core error types
mod errors;
pub use errors::{SessionError, Result};

// Re-export the main API for convenience
pub use api::*;

// Re-export SessionManager for direct access
pub use manager::SessionManager;

// Prelude module for common imports
pub mod prelude {
    pub use crate::api::*;
    pub use crate::errors::{SessionError, Result};
    pub use crate::events::{SessionEvent, EventBus};
    pub use crate::manager::SessionManager;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_library_compiles() {
        // Basic smoke test to ensure the library structure compiles
        assert!(true);
    }
} 