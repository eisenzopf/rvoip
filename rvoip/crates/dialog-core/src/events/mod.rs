//! Event types for dialog-core
//!
//! This module defines events for dialog state changes and coordination
//! with the session layer.

pub mod dialog_events;
pub mod session_coordination;

// Re-export main event types
pub use dialog_events::DialogEvent;
pub use session_coordination::SessionCoordinationEvent; 