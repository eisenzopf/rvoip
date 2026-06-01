//! Dialog Manager Module
//!
//! This module provides the main DialogManager and all its supporting functionality,
//! organized into focused submodules for better maintainability and separation of concerns.

// Core dialog manager implementation
pub mod core;

/// Maximum dialog transaction-event dispatch workers accepted by configuration.
pub const MAX_DIALOG_EVENT_DISPATCH_WORKERS: usize = 64;

// **NEW**: Unified dialog manager (replaces client/server split)
pub mod unified;

// Helper modules for dialog operations
pub mod dialog_operations;
pub mod event_processing;
pub mod identity_verify;
pub mod message_routing;
pub mod outbound_flow;
pub mod protocol_handlers;
pub mod request_lifecycle;
pub mod response_lifecycle;
pub mod session_coordination;
pub mod session_timer;
pub mod transaction_integration;
pub mod utils;

// Transaction integration (organized by module)
pub mod transaction {
    pub use super::transaction_integration::*;
}

// Re-export the main DialogManager
pub use core::DialogManager;

// Re-export the STIR/SHAKEN verify-decision type so both publish
// paths (events/adapter.rs and events/event_hub.rs) can address it
// without reaching into `manager::core`.
pub use core::IdentityVerificationDecision;

// Re-export commonly used types from submodules
pub use dialog_operations::{DialogLookup, DialogStore};
pub use identity_verify::{
    IdentityHeaderValue, IdentityVerificationStatus, PASSporTSigner, PASSporTVerifier,
    PassportClaimSummary, SharedSigner, SharedVerifier, SignerErrorKind, VerificationOutcome,
    VerificationPolicy,
};
pub use message_routing::{DialogMatcher, MessageRouter};
pub use protocol_handlers::{MethodHandler, ProtocolHandlers};
pub use request_lifecycle::RequestLifecycle;
pub use response_lifecycle::ResponseLifecycle;
pub use session_coordination::{EventSender, SessionCoordinator};
pub use transaction_integration::{TransactionHelpers, TransactionIntegration};
pub use utils::{MessageExtensions, SourceExtractor};

// Re-export main types
pub use unified::UnifiedDialogManager;
