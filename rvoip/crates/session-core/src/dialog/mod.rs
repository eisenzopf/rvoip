//! Dialog management module
//!
//! This module handles SIP dialogs according to RFC 3261. A dialog represents
//! a peer-to-peer SIP relationship between two UAs that persists for some time.
//! The dialog state is identified by the Call-ID, From tag, and To tag.
//!
//! **ARCHITECTURAL NOTE**: CallLifecycleCoordinator was moved from this module
//! to the session layer to fix RFC 3261 separation violations. Dialog layer
//! now focuses purely on SIP protocol dialog state management.

pub mod dialog_id;
pub mod dialog_impl;
pub mod dialog_state;
pub mod dialog_utils;
pub mod recovery;

// Refactored dialog manager modules
pub mod manager;
pub mod event_processing;
pub mod transaction_handling;
pub mod dialog_operations;
pub mod sdp_handling;
pub mod recovery_manager;
pub mod testing;
pub mod transaction_coordination;

// Export recovery functions
pub use recovery::{needs_recovery, begin_recovery, complete_recovery, abandon_recovery, send_recovery_options};

// Re-export the main types for backward compatibility
pub use manager::DialogManager;
pub use dialog_id::DialogId;
pub use dialog_impl::Dialog;
pub use dialog_state::DialogState;
pub use transaction_coordination::TransactionCoordinator;
pub use recovery::{RecoveryConfig, RecoveryMetrics}; 