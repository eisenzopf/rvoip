//! Dialog Manager Module
//!
//! This module provides the main DialogManager and all its supporting functionality,
//! organized into focused submodules for better maintainability and separation of concerns.
//!
//! ## Submodules
//!
//! - `core`: Main DialogManager implementation and lifecycle management
//! - `dialog_operations`: Dialog storage, lookup, and CRUD operations  
//! - `protocol_handlers`: SIP method handlers (INVITE, BYE, etc.)
//! - `message_routing`: Routes incoming messages to appropriate dialogs
//! - `transaction_integration`: Integration with transaction-core for reliability
//! - `session_coordination`: Coordination with session-core for call management
//! - `utils`: Utility functions for message processing and source extraction
//!
//! ## Architecture
//!
//! The DialogManager serves as the central coordinator between:
//! - SIP transport layer (via transaction-core)
//! - Dialog state management
//! - Session coordination (via session-core)
//! - API layer (DialogClient/DialogServer)

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