//! Session history tracking for debugging and analysis

use std::collections::VecDeque;
use std::time::{Duration, Instant};
use serde::{Serialize, Deserialize};
use crate::state_table::{EventType, Guard, Action, EventTemplate};
use crate::types::CallState;

/// Configuration for history tracking
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct HistoryConfig {
    /// Maximum number of transitions to keep per session
    pub max_transitions: usize,
    
    /// Enable history tracking
    pub enabled: bool,
    
    /// Include action details in history
    pub track_actions: bool,
    
    /// Include guard evaluation results
    pub track_guards: bool,
}

impl Default for HistoryConfig {
    fn default() -> Self {
        Self {
            max_transitions: 50,
            #[cfg(debug_assertions)]
            enabled: true,
            #[cfg(not(debug_assertions))]
            enabled: false,
            track_actions: true,
            track_guards: false,
        }
    }
}

/// Record of a single state transition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransitionRecord {
    /// When the transition occurred (milliseconds since UNIX epoch)
    #[serde(skip, default = "Instant::now")]
    pub timestamp: Instant,
    
    /// Timestamp for serialization (milliseconds since UNIX epoch)
    pub timestamp_ms: u64,
    
    /// Monotonic sequence number
    pub sequence: u64,
    
    /// State before transition
    pub from_state: CallState,
    
    /// Event that triggered transition
    pub event: EventType,
    
    /// State after transition (None if no change)
    pub to_state: Option<CallState>,
    
    /// Guards that were evaluated
    pub guards_evaluated: Vec<GuardResult>,
    
    /// Actions that were executed
    pub actions_executed: Vec<ActionRecord>,
    
    /// Events published as result
    pub events_published: Vec<EventTemplate>,
    
    /// Duration of transition processing
    pub duration_ms: u64,
    
    /// Any errors that occurred
    pub errors: Vec<String>,
}

/// Result of guard evaluation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GuardResult {
    pub guard: Guard,
    pub passed: bool,
    pub evaluation_time_us: u64,
}

/// Record of action execution
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionRecord {
    pub action: Action,
    pub success: bool,
    pub execution_time_us: u64,
    pub error: Option<String>,
}

/// Session history with ring buffer
#[derive(Debug, Clone)]
pub struct SessionHistory {
    /// Ring buffer of transitions
    transitions: VecDeque<TransitionRecord>,
    
    /// Configuration
    config: HistoryConfig,
    
    /// Next sequence number
    next_sequence: u64,
    
    /// Statistics
    pub total_transitions: u64,
    pub total_errors: u64,
    pub session_created: Instant,
    pub last_activity: Instant,
}

impl SessionHistory {
    /// Create new session history
    pub fn new(config: HistoryConfig) -> Self {
        Self {
            transitions: VecDeque::with_capacity(config.max_transitions),
            config,
            next_sequence: 0,
            total_transitions: 0,
            total_errors: 0,
            session_created: Instant::now(),
            last_activity: Instant::now(),
        }
    }
    
    /// Record a transition
    pub fn record_transition(&mut self, mut record: TransitionRecord) {
        if !self.config.enabled {
            return;
        }
        
        // Filter guards/actions based on config
        if !self.config.track_guards {
            record.guards_evaluated.clear();
        }
        if !self.config.track_actions {
            record.actions_executed.clear();
        }
        
        // Update statistics
        self.total_transitions += 1;
        if !record.errors.is_empty() {
            self.total_errors += 1;
        }
        self.last_activity = Instant::now();
        
        // Add sequence number
        record.sequence = self.next_sequence;
        self.next_sequence += 1;
        
        // Maintain ring buffer size
        if self.transitions.len() >= self.config.max_transitions {
            self.transitions.pop_front();
        }
        
        self.transitions.push_back(record);
    }
    
    /// Get recent transitions
    pub fn get_recent(&self, count: usize) -> Vec<TransitionRecord> {
        self.transitions
            .iter()
            .rev()
            .take(count)
            .cloned()
            .collect()
    }
    
    /// Get transitions involving a specific state
    pub fn get_by_state(&self, state: CallState) -> Vec<TransitionRecord> {
        self.transitions
            .iter()
            .filter(|t| t.from_state == state || t.to_state == Some(state))
            .cloned()
            .collect()
    }
    
    /// Get transitions with errors
    pub fn get_errors(&self) -> Vec<TransitionRecord> {
        self.transitions
            .iter()
            .filter(|t| !t.errors.is_empty())
            .cloned()
            .collect()
    }
    
    /// Get transition count for a specific event type
    pub fn count_by_event(&self, event_type: &EventType) -> usize {
        self.transitions
            .iter()
            .filter(|t| std::mem::discriminant(&t.event) == std::mem::discriminant(event_type))
            .count()
    }
    
    /// Get average transition duration
    pub fn average_duration_ms(&self) -> f64 {
        if self.transitions.is_empty() {
            return 0.0;
        }
        
        let total: u64 = self.transitions.iter().map(|t| t.duration_ms).sum();
        total as f64 / self.transitions.len() as f64
    }
    
    /// Clear history
    pub fn clear(&mut self) {
        self.transitions.clear();
        self.next_sequence = 0;
        self.total_transitions = 0;
        self.total_errors = 0;
        self.last_activity = Instant::now();
    }
    
    /// Export history as JSON
    pub fn export_json(&self) -> String {
        serde_json::to_string_pretty(&self.transitions).unwrap_or_else(|_| "[]".to_string())
    }
    
    /// Export history as CSV
    pub fn export_csv(&self) -> String {
        let mut csv = String::from("sequence,timestamp_ms,from_state,event,to_state,duration_ms,errors\n");
        
        for t in &self.transitions {
            csv.push_str(&format!(
                "{},{},{:?},\"{:?}\",{:?},{},{}\n",
                t.sequence,
                t.timestamp.elapsed().as_millis(),
                t.from_state,
                t.event,
                t.to_state.map(|s| format!("{:?}", s)).unwrap_or_else(|| "None".to_string()),
                t.duration_ms,
                t.errors.join(";")
            ));
        }
        
        csv
    }
    
    /// Get session age
    pub fn session_age(&self) -> Duration {
        self.session_created.elapsed()
    }
    
    /// Get time since last activity
    pub fn idle_time(&self) -> Duration {
        self.last_activity.elapsed()
    }
    
    /// Check if session is idle
    pub fn is_idle(&self, threshold: Duration) -> bool {
        self.idle_time() > threshold
    }
    
    /// Get error rate
    pub fn error_rate(&self) -> f32 {
        if self.total_transitions == 0 {
            return 0.0;
        }
        self.total_errors as f32 / self.total_transitions as f32
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::{SystemTime, UNIX_EPOCH};
    
    #[test]
    fn test_ring_buffer_limit() {
        let config = HistoryConfig {
            max_transitions: 3,
            enabled: true,
            ..Default::default()
        };
        
        let mut history = SessionHistory::new(config);
        
        // Add 5 transitions to a buffer with max 3
        for i in 0..5 {
            let now = Instant::now();
            let record = TransitionRecord {
                timestamp: now,
                timestamp_ms: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
                sequence: 0,
                from_state: CallState::Idle,
                event: EventType::MakeCall { target: format!("test{}", i) },
                to_state: Some(CallState::Initiating),
                guards_evaluated: vec![],
                actions_executed: vec![],
                events_published: vec![],
                duration_ms: 10,
                errors: vec![],
            };
            history.record_transition(record);
        }
        
        // Should only have 3 transitions (the last 3)
        assert_eq!(history.transitions.len(), 3);
        assert_eq!(history.total_transitions, 5);
        
        // Check that we have the last 3 (sequences 2, 3, 4)
        let recent = history.get_recent(10);
        assert_eq!(recent.len(), 3);
        assert_eq!(recent[0].sequence, 4);
        assert_eq!(recent[1].sequence, 3);
        assert_eq!(recent[2].sequence, 2);
    }
    
    #[test]
    fn test_error_tracking() {
        let mut history = SessionHistory::new(HistoryConfig::default());
        
        // Add some transitions with and without errors
        for i in 0..5 {
            let now = Instant::now();
            let mut record = TransitionRecord {
                timestamp: now,
                timestamp_ms: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
                sequence: 0,
                from_state: CallState::Active,
                event: EventType::HangupCall,
                to_state: Some(CallState::Terminating),
                guards_evaluated: vec![],
                actions_executed: vec![],
                events_published: vec![],
                duration_ms: 10,
                errors: vec![],
            };
            
            if i % 2 == 0 {
                record.errors.push(format!("Error {}", i));
            }
            
            history.record_transition(record);
        }
        
        assert_eq!(history.total_transitions, 5);
        assert_eq!(history.total_errors, 3);
        assert_eq!(history.get_errors().len(), 3);
        assert_eq!(history.error_rate(), 0.6);
    }
    
    #[test]
    fn test_state_filtering() {
        let mut history = SessionHistory::new(HistoryConfig::default());
        
        // Add transitions through different states
        let states = vec![
            (CallState::Idle, CallState::Initiating),
            (CallState::Initiating, CallState::Ringing),
            (CallState::Ringing, CallState::Active),
            (CallState::Active, CallState::OnHold),
            (CallState::OnHold, CallState::Active),
        ];
        
        for (from, to) in states {
            let record = TransitionRecord {
                timestamp: Instant::now(),
                timestamp_ms: SystemTime::now()
                    .duration_since(UNIX_EPOCH)
                    .unwrap_or_default()
                    .as_millis() as u64,
                sequence: 0,
                from_state: from,
                event: EventType::MakeCall { target: "test".to_string() },
                to_state: Some(to),
                guards_evaluated: vec![],
                actions_executed: vec![],
                events_published: vec![],
                duration_ms: 10,
                errors: vec![],
            };
            history.record_transition(record);
        }
        
        // Should find 2 transitions involving Active state
        let active_transitions = history.get_by_state(CallState::Active);
        assert_eq!(active_transitions.len(), 3); // Ringing->Active, Active->OnHold, OnHold->Active
    }
}