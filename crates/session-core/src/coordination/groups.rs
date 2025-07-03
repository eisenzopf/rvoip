//! Session Groups
//!
//! Simplified session grouping for basic coordination.

use std::collections::HashMap;
use crate::api::types::SessionId;
use crate::errors::Result;

/// Simple session group manager
#[derive(Debug, Default)]
pub struct SessionGroups {
    groups: HashMap<String, Vec<SessionId>>,
}

impl SessionGroups {
    pub fn new() -> Self {
        Self::default()
    }

    pub fn add_to_group(&mut self, group_name: &str, session_id: SessionId) -> Result<()> {
        self.groups.entry(group_name.to_string()).or_default().push(session_id);
        Ok(())
    }

    pub fn remove_from_group(&mut self, group_name: &str, session_id: &SessionId) -> Result<()> {
        if let Some(sessions) = self.groups.get_mut(group_name) {
            sessions.retain(|id| id != session_id);
        }
        Ok(())
    }

    pub fn get_group_sessions(&self, group_name: &str) -> Vec<SessionId> {
        self.groups.get(group_name).cloned().unwrap_or_default()
    }
} 