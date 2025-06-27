//! Resource Limits
//!
//! Simple resource limit management for sessions.

use crate::errors::Result;

/// Simple resource limits
#[derive(Debug, Clone)]
pub struct ResourceLimits {
    pub max_sessions: usize,
    pub max_media_ports: usize,
}

impl Default for ResourceLimits {
    fn default() -> Self {
        Self {
            max_sessions: 1000,
            max_media_ports: 2000,
        }
    }
}

/// Resource manager
#[derive(Debug)]
pub struct ResourceManager {
    limits: ResourceLimits,
    current_sessions: usize,
    current_media_ports: usize,
}

impl ResourceManager {
    pub fn new(limits: ResourceLimits) -> Self {
        Self {
            limits,
            current_sessions: 0,
            current_media_ports: 0,
        }
    }

    pub fn can_create_session(&self) -> bool {
        self.current_sessions < self.limits.max_sessions
    }

    pub fn allocate_session(&mut self) -> Result<()> {
        if self.can_create_session() {
            self.current_sessions += 1;
            Ok(())
        } else {
            Err(crate::errors::SessionError::ResourceLimitExceeded("Max sessions reached".to_string()))
        }
    }

    pub fn deallocate_session(&mut self) -> Result<()> {
        if self.current_sessions > 0 {
            self.current_sessions -= 1;
        }
        Ok(())
    }
}

impl Default for ResourceManager {
    fn default() -> Self {
        Self::new(ResourceLimits::default())
    }
} 