//! Event types for dialog-core
//!
//! This module defines events for dialog state changes and coordination
//! with the session layer.

pub mod dialog_events;
pub mod session_coordination;
pub mod adapter;
pub mod event_hub;

// Re-export main event types
pub use dialog_events::DialogEvent;
pub use session_coordination::SessionCoordinationEvent;
pub use adapter::DialogEventAdapter;
pub use event_hub::DialogEventHub; 