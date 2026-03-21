//! State machine executor module (state-table feature)
//!
//! This module provides the state machine that processes events through
//! the state table. The full executor and actions modules require session_store
//! and adapter modules which will be integrated in a future phase.
//!
//! Currently provides:
//! - Type definitions (ProcessEventResult, SessionEvent)
//! - Guard evaluation logic
//! - Effect handling
//! - Helper types

pub mod guards;
pub mod effects;

use crate::state_table::{Action, EventTemplate, Transition};
use crate::state_table::types::CallState;

/// Result of processing an event through the state machine
#[derive(Debug, Clone)]
pub struct ProcessEventResult {
    /// The old state before processing
    pub old_state: CallState,
    /// The new state after processing
    pub next_state: Option<CallState>,
    /// The transition that was executed (if any)
    pub transition: Option<Transition>,
    /// Actions that were executed
    pub actions_executed: Vec<Action>,
    /// Events that were published
    pub events_published: Vec<EventTemplate>,
}
