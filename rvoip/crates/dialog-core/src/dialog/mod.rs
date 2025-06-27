//! Core dialog types and functionality
//!
//! This module contains the core dialog types and operations for RFC 3261 SIP dialogs:
//!
//! - [`DialogId`]: Unique UUID-based identifiers for dialogs
//! - [`Dialog`]: Main dialog implementation with state management
//! - [`DialogState`]: Dialog lifecycle states (Initial, Early, Confirmed, etc.)
//! - [`dialog_utils`]: Utility functions for SIP parsing and URI handling
//!
//! ## Dialog Lifecycle
//!
//! ```text
//! Initial → Early → Confirmed → Terminated
//!    ↓        ↓        ↓          ↓
//!  INVITE   18x      2xx       BYE
//!  sent     recv'd   recv'd    sent/recv'd
//! ```
//!
//! ## Usage
//!
//! Dialogs are typically created and managed through the [`DialogManager`](crate::manager::DialogManager)
//! or the high-level [`DialogClient`](crate::api::DialogClient) and [`DialogServer`](crate::api::DialogServer) APIs.

pub mod dialog_id;
pub mod dialog_impl;
pub mod dialog_state;
pub mod dialog_utils;

// Re-export main types
pub use dialog_id::DialogId;
pub use dialog_impl::Dialog;
pub use dialog_state::DialogState;
pub use dialog_utils::*; 