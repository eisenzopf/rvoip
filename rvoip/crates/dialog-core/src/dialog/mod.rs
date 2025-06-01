//! Core dialog types and functionality
//!
//! This module contains the core dialog types including DialogId, Dialog,
//! DialogState, and utility functions for dialog management.

pub mod dialog_id;
pub mod dialog_impl;
pub mod dialog_state;
pub mod dialog_utils;

// Re-export main types
pub use dialog_id::DialogId;
pub use dialog_impl::Dialog;
pub use dialog_state::DialogState;
pub use dialog_utils::*; 