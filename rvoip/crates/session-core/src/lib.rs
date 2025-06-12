// RVOIP Session Core Library
//
// This library provides Session Initiation Protocol (SIP) session and dialog management 
// for the RVOIP stack. It serves as the middle layer between low-level SIP transaction 
// processing and high-level application logic.

pub mod api;
pub mod session;
pub mod dialog;        // NEW - dialog-core integration
pub mod media;         // EXISTING - media-core integration
pub mod manager;       // SIMPLIFIED - orchestration only
pub mod coordination;
pub mod bridge;
pub mod conference;    // NEW - Conference functionality
pub mod coordinator;

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
    pub use crate::manager::events::{SessionEvent, SessionEventProcessor};
    pub use crate::manager::SessionManager;
    pub use crate::dialog::DialogManager;  // NEW
    pub use crate::conference::prelude::*; // NEW - Conference functionality
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

// Feature flags
#[cfg(feature = "testing")]
pub mod testing {
    //! Testing utilities and mocks
    pub use crate::manager::testing::*;
}

// Version information
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

/// Initialize the session core library
/// 
/// This should be called once at application startup to initialize
/// any global state or resources.
pub fn init() {
    // Initialize logging if not already done
    let _ = tracing_subscriber::fmt::try_init();
    
    // Any other global initialization
    tracing::info!("RVoIP Session Core v{} initialized", VERSION);
} 