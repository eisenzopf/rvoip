use std::collections::HashMap;
use std::time::Instant;

/// Availability tracker for monitoring agent availability
pub struct AvailabilityTracker {
    /// Last activity time for each agent
    last_activity: HashMap<String, Instant>,
}

impl AvailabilityTracker {
    /// Create a new availability tracker
    pub fn new() -> Self {
        Self {
            last_activity: HashMap::new(),
        }
    }
    
    /// Update agent activity timestamp
    pub fn update_activity(&mut self, agent_id: String) {
        self.last_activity.insert(agent_id, Instant::now());
    }
    
    /// Check if agent is considered active
    pub fn is_agent_active(&self, agent_id: &str, timeout_secs: u64) -> bool {
        if let Some(last_activity) = self.last_activity.get(agent_id) {
            last_activity.elapsed().as_secs() < timeout_secs
        } else {
            false
        }
    }
}

impl Default for AvailabilityTracker {
    fn default() -> Self {
        Self::new()
    }
} 