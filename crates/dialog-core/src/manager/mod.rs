//! Dialog Manager Module
//!
//! This module provides the main DialogManager and all its supporting functionality,
//! organized into focused submodules for better maintainability and separation of concerns.

// Core dialog manager implementation
pub mod core;

// **NEW**: Unified dialog manager (replaces client/server split)
pub mod unified;

// Helper modules for dialog operations
pub mod dialog_operations;
pub mod protocol_handlers;
pub mod transaction_integration;
pub mod response_lifecycle;
pub mod event_processing;
pub mod session_coordination;
pub mod message_routing;
pub mod utils;

// Transaction integration (organized by module)
pub mod transaction {
    pub use super::transaction_integration::*;
}

// Re-export the main DialogManager
pub use core::DialogManager;

// Re-export commonly used types from submodules
pub use dialog_operations::{DialogStore, DialogLookup};
pub use protocol_handlers::{ProtocolHandlers, MethodHandler};
pub use message_routing::{MessageRouter, DialogMatcher};
pub use transaction_integration::{TransactionIntegration, TransactionHelpers};
pub use response_lifecycle::ResponseLifecycle;
pub use session_coordination::{SessionCoordinator, EventSender};
pub use utils::{MessageExtensions, SourceExtractor};

// Re-export main types
pub use unified::UnifiedDialogManager; 