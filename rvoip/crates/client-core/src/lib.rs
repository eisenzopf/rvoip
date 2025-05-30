//! Client-core: High-level SIP client coordination layer
//!
//! This crate provides a high-level API for SIP client applications by delegating
//! to session-core for all SIP session and media orchestration.
//!
//! ## Proper Layer Separation
//! ```text
//! client-core -> session-core -> {transaction-core, media-core, sip-transport, sip-core}
//! ```
//!
//! Client-core focuses on:
//! - User-friendly call management API
//! - Event handling for UI integration
//! - Configuration management
//! - Call state mapping and tracking
//!
//! All SIP protocol details, media management, and infrastructure
//! are handled by session-core and lower layers.

pub mod client;
pub mod call;
pub mod registration;
pub mod events;
pub mod error;

// Public API exports (only high-level client-core types)
pub use client::{ClientManager, ClientConfig, ClientStats};
pub use call::{CallState, CallInfo, CallId, CallDirection, CallStats};
pub use registration::{RegistrationConfig, RegistrationInfo, RegistrationStatus};
pub use events::{
    ClientEventHandler, ClientEvent, IncomingCallInfo, CallStatusInfo,
    RegistrationStatusInfo, CallAction, MediaEventType
};
pub use error::{ClientError, ClientResult};

// Re-export commonly used types from session-core (for convenience)
pub use rvoip_session_core::{SessionId, SessionState};

/// Client-core version information
pub const VERSION: &str = env!("CARGO_PKG_VERSION");

#[cfg(test)]
mod tests {
    use super::*;

    #[tokio::test]
    async fn test_client_core_compiles() {
        // Basic compilation test
        assert!(true);
    }
} 