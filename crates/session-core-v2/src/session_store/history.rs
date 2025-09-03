use std::collections::VecDeque;
use std::time::Instant;
use serde::{Deserialize, Serialize};

use crate::state_table::CallState;

/// Record of a single state transition
#[derive(Debug, Clone)]
pub struct TransitionRecord {
    pub timestamp: Instant,
    pub from_state: CallState,
    pub to_state: CallState,
    pub duration_ms: u64,
}

/// Session history tracking
#[derive(Debug, Clone)]
pub struct SessionHistory {
    transitions: VecDeque<TransitionRecord>,
    max_size: usize,
}

impl SessionHistory {
    /// Create a new history tracker
    pub fn new(max_size: usize) -> Self {
        Self {
            transitions: VecDeque::with_capacity(max_size),
            max_size,
        }
    }
    
    /// Record a state transition
    pub fn record_transition(
        &mut self,
        from_state: CallState,
        to_state: CallState,
        timestamp: Instant,
    ) {
        // Calculate duration if we have a previous transition
        let duration_ms = if let Some(last) = self.transitions.back() {
            timestamp.duration_since(last.timestamp).as_millis() as u64
        } else {
            0
        };
        
        // Add new record
        let record = TransitionRecord {
            timestamp,
            from_state,
            to_state,
            duration_ms,
        };
        
        // Maintain max size
        if self.transitions.len() >= self.max_size {
            self.transitions.pop_front();
        }
        
        self.transitions.push_back(record);
    }
    
    /// Get all transition records
    pub fn get_history(&self) -> Vec<TransitionRecord> {
        self.transitions.iter().cloned().collect()
    }
    
    /// Get the last N transitions
    pub fn get_recent(&self, count: usize) -> Vec<TransitionRecord> {
        self.transitions
            .iter()
            .rev()
            .take(count)
            .rev()
            .cloned()
            .collect()
    }
    
    /// Clear history
    pub fn clear(&mut self) {
        self.transitions.clear();
    }
}