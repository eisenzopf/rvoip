//! Dialog coordination for session-core
//!
//! This module provides session-level coordination with dialog-core,
//! maintaining proper architectural separation.
//!
//! **ARCHITECTURE COMPLIANCE**: session-core only uses dialog-core APIs.
//! All SIP protocol work is delegated to dialog-core.

// Re-export dialog types from dialog-core for convenience
pub use rvoip_dialog_core::{Dialog, DialogState, DialogId, DialogManager, SessionCoordinationEvent}; 