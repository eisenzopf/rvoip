// Dialog module for handling SIP dialogs
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
pub mod call_lifecycle;

// Export recovery functions
pub use recovery::{needs_recovery, begin_recovery, complete_recovery, abandon_recovery, send_recovery_options};

// Re-export the main types for backward compatibility
pub use manager::DialogManager;
pub use dialog_id::DialogId;
pub use dialog_impl::Dialog;
pub use dialog_state::DialogState;
pub use transaction_coordination::TransactionCoordinator;
pub use call_lifecycle::CallLifecycleCoordinator; 