//! Bridge Implementation
//!
//! Handles bridging multiple sessions together.

use std::collections::HashSet;
use crate::api::types::SessionId;
use crate::errors::Result;

/// Bridge for connecting multiple sessions
#[derive(Debug)]
pub struct SessionBridge {
    id: String,
    sessions: HashSet<SessionId>,
    active: bool,
}

impl SessionBridge {
    pub fn new(id: String) -> Self {
        Self {
            id,
            sessions: HashSet::new(),
            active: false,
        }
    }

    pub fn add_session(&mut self, session_id: SessionId) -> Result<()> {
        self.sessions.insert(session_id);
        tracing::debug!("Added session to bridge {}: {} sessions", self.id, self.sessions.len());
        Ok(())
    }

    pub fn remove_session(&mut self, session_id: &SessionId) -> Result<()> {
        self.sessions.remove(session_id);
        tracing::debug!("Removed session from bridge {}: {} sessions", self.id, self.sessions.len());
        Ok(())
    }

    pub fn start(&mut self) -> Result<()> {
        self.active = true;
        tracing::info!("Started bridge: {}", self.id);
        Ok(())
    }

    pub fn stop(&mut self) -> Result<()> {
        self.active = false;
        tracing::info!("Stopped bridge: {}", self.id);
        Ok(())
    }

    pub fn is_active(&self) -> bool {
        self.active
    }

    pub fn session_count(&self) -> usize {
        self.sessions.len()
    }
} 