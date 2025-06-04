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
//! - `transaction_integration`: Integration with transaction-core for reliability (modularized)
//! - `transaction`: Modular transaction integration components
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
pub mod core;
pub mod dialog_operations;
pub mod protocol_handlers;
pub mod message_routing;
pub mod transaction_integration;
pub mod transaction;
pub mod session_coordination;
pub mod utils;
mod event_processing;

// Re-export the main DialogManager
pub use core::DialogManager;

// Re-export commonly used types from submodules
pub use dialog_operations::{DialogStore, DialogLookup};
pub use protocol_handlers::{ProtocolHandlers, MethodHandler};
pub use message_routing::{MessageRouter, DialogMatcher};
pub use transaction_integration::{TransactionIntegration, TransactionHelpers};
pub use transaction::{TransactionIntegration as NewTransactionIntegration, TransactionHelpers as NewTransactionHelpers};
pub use session_coordination::{SessionCoordinator, EventSender};
pub use utils::{MessageExtensions, SourceExtractor}; 