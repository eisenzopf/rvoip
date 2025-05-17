// Dialog module for handling SIP dialogs
pub mod dialog_id;
pub mod dialog_impl;
pub mod dialog_manager;
pub mod dialog_state;
pub mod dialog_utils;
pub mod recovery;

#[cfg(test)]
mod tests;

pub use dialog_id::DialogId;
pub use dialog_impl::Dialog;
pub use dialog_manager::DialogManager;
pub use dialog_state::DialogState;
// Export recovery functions
pub use recovery::{needs_recovery, begin_recovery, complete_recovery, abandon_recovery, send_recovery_options}; 