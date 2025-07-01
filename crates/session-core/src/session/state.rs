//! Session State Management
//!
//! Handles session state transitions and validation.

use crate::api::types::CallState;
use crate::errors::Result;

/// State transition validator
pub struct StateManager;

impl StateManager {
    pub fn can_transition(from: &CallState, to: &CallState) -> bool {
        use CallState::*;
        
        match (from, to) {
            (Initiating, Ringing) => true,
            (Ringing, Active) => true,
            (Active, OnHold) => true,
            (OnHold, Active) => true,
            (_, Terminated) => true,
            (_, Failed(_)) => true,
            _ => false,
        }
    }

    pub fn validate_transition(from: &CallState, to: &CallState) -> Result<()> {
        if Self::can_transition(from, to) {
            Ok(())
        } else {
            Err(crate::errors::SessionError::invalid_state(
                &format!("Invalid state transition: {:?} -> {:?}", from, to)
            ))
        }
    }
} 