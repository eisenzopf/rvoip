//! Event types for dialog-core
//!
//! This module defines events for dialog state changes and coordination
//! with the session layer.

pub mod adapter;
pub mod dialog_events;
pub mod event_hub;
pub mod session_coordination;

// Re-export main event types
pub use adapter::DialogEventAdapter;
pub use dialog_events::DialogEvent;
pub use event_hub::DialogEventHub;
pub use session_coordination::{FlowFailureReason, SessionCoordinationEvent};
