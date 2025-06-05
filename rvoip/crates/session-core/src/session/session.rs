//! Session Implementation
//!
//! Core session handling logic.

use crate::api::types::{SessionId, CallState};
use crate::errors::Result;

/// Internal session implementation details
#[derive(Debug, Clone)]
pub struct SessionImpl {
    pub id: SessionId,
    pub state: CallState,
}

impl SessionImpl {
    pub fn new(id: SessionId) -> Self {
        Self {
            id,
            state: CallState::Initiating,
        }
    }

    pub fn update_state(&mut self, new_state: CallState) -> Result<()> {
        tracing::debug!("Session {} state: {:?} -> {:?}", self.id, self.state, new_state);
        self.state = new_state;
        Ok(())
    }
} 