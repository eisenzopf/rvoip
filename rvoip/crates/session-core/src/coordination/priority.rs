//! Priority Handling
//!
//! Simplified priority management for sessions.

use crate::api::types::SessionId;
use crate::errors::Result;

/// Simple priority levels
#[derive(Debug, Clone, Copy, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Priority {
    Low = 1,
    Normal = 2,
    High = 3,
    Critical = 4,
}

/// Priority manager for sessions
#[derive(Debug, Default)]
pub struct PriorityManager {
    session_priorities: std::collections::HashMap<SessionId, Priority>,
}

impl PriorityManager {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn set_priority(&mut self, session_id: SessionId, priority: Priority) -> Result<()> {
        self.session_priorities.insert(session_id, priority);
        Ok(())
    }

    pub fn get_priority(&self, session_id: &SessionId) -> Priority {
        self.session_priorities.get(session_id).cloned().unwrap_or(Priority::Normal)
    }

    pub fn remove_session(&mut self, session_id: &SessionId) -> Result<()> {
        self.session_priorities.remove(session_id);
        Ok(())
    }
} 