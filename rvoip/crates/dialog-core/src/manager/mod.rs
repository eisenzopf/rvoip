//! Dialog Manager Module
//!
//! This module provides the main DialogManager and all its supporting functionality,
//! organized into focused submodules for better maintainability and separation of concerns.

// Core dialog manager implementation
mod core;
mod dialog_operations;
mod protocol_handlers;
mod message_routing;
mod transaction_integration;
mod session_coordination;
mod utils;

// Re-export the main DialogManager
pub use core::DialogManager;

// Re-export commonly used types from submodules
pub use dialog_operations::{DialogStore, DialogLookup};
pub use protocol_handlers::{ProtocolHandlers, MethodHandler};
pub use message_routing::{MessageRouter, DialogMatcher};
pub use transaction_integration::{TransactionIntegration, TransactionHelpers};
pub use session_coordination::{SessionCoordinator, EventSender};
pub use utils::{MessageExtensions, SourceExtractor}; 