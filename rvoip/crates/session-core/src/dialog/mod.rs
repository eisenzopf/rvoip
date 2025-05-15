// Dialog module - SIP dialog implementation as defined in RFC 3261
mod dialog_id;
mod dialog_impl;
mod dialog_utils;
mod dialog_manager;
mod dialog_state;

// Re-export main types
pub use dialog_id::DialogId;
pub use dialog_impl::Dialog;
pub use dialog_manager::DialogManager;
pub use dialog_utils::{extract_tag, extract_uri};
pub use dialog_state::DialogState;

#[cfg(test)]
mod tests; 