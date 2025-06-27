//! Error types for dialog-core
//! 
//! This module defines all error types used throughout the dialog-core crate,
//! including dialog errors, recovery errors, and conversion utilities.

pub mod dialog_errors;
pub mod recovery_errors;

// Re-export main error types
pub use dialog_errors::{DialogError, DialogResult};
pub use recovery_errors::{RecoveryError, RecoveryResult}; 