# High Priority Features Implementation Plan

## Overview
This plan covers implementation of HIGH priority features from MISSING_FEATURES.md:
1. Event History & Debugging 
2. Session Cleanup & Resource Management

**Estimated Time**: 2-3 days
**Risk**: Low - These are additive features that don't change existing APIs

## 1. Event History & Debugging Implementation

### 1.1 Session History Tracking

#### Data Structures
```rust
// File: src/session_store/history.rs

use std::collections::VecDeque;
use std::time::Instant;
use serde::{Serialize, Deserialize};

/// Configuration for history tracking
#[derive(Debug, Clone)]
pub struct HistoryConfig {
    /// Maximum number of transitions to keep per session
    pub max_transitions: usize,  // Default: 50
    
    /// Enable history tracking
    pub enabled: bool,  // Default: true in debug, false in release
    
    /// Include action details in history
    pub track_actions: bool,  // Default: true
    
    /// Include guard evaluation results
    pub track_guards: bool,  // Default: false (verbose)
}

/// Record of a single state transition
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransitionRecord {
    /// When the transition occurred
    pub timestamp: Instant,
    
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
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    
    pub fn record_transition(&mut self, record: TransitionRecord) {
        if !self.config.enabled {
            return;
        }
        
        // Update statistics
        self.total_transitions += 1;
        if !record.errors.is_empty() {
            self.total_errors += 1;
        }
        self.last_activity = Instant::now();
        
        // Add sequence number
        let mut record = record;
        record.sequence = self.next_sequence;
        self.next_sequence += 1;
        
        // Maintain ring buffer size
        if self.transitions.len() >= self.config.max_transitions {
            self.transitions.pop_front();
        }
        
        self.transitions.push_back(record);
    }
    
    pub fn get_recent(&self, count: usize) -> Vec<TransitionRecord> {
        self.transitions
            .iter()
            .rev()
            .take(count)
            .cloned()
            .collect()
    }
    
    pub fn get_by_state(&self, state: CallState) -> Vec<TransitionRecord> {
        self.transitions
            .iter()
            .filter(|t| t.from_state == state || t.to_state == Some(state))
            .cloned()
            .collect()
    }
    
    pub fn get_errors(&self) -> Vec<TransitionRecord> {
        self.transitions
            .iter()
            .filter(|t| !t.errors.is_empty())
            .cloned()
            .collect()
    }
    
    pub fn clear(&mut self) {
        self.transitions.clear();
        self.next_sequence = 0;
        self.total_transitions = 0;
        self.total_errors = 0;
    }
    
    pub fn export_json(&self) -> String {
        serde_json::to_string_pretty(&self.transitions).unwrap_or_default()
    }
}
```

#### Integration with SessionState
```rust
// Modify src/session_store/state.rs

pub struct SessionState {
    // ... existing fields ...
    
    /// Optional history tracking
    pub history: Option<SessionHistory>,
}

impl SessionState {
    pub fn new_with_history(session_id: SessionId, role: Role, config: HistoryConfig) -> Self {
        let mut state = Self::new(session_id, role);
        state.history = Some(SessionHistory::new(config));
        state
    }
    
    pub fn record_transition(&mut self, record: TransitionRecord) {
        if let Some(ref mut history) = self.history {
            history.record_transition(record);
        }
    }
}
```

#### Integration with StateMachine
```rust
// Modify src/state_machine/executor.rs

impl StateMachine {
    pub async fn process_event(&self, session_id: &SessionId, event: EventType) -> Result<ProcessEventResult> {
        let start_time = Instant::now();
        let mut guards_evaluated = Vec::new();
        let mut actions_executed = Vec::new();
        let mut errors = Vec::new();
        
        // ... existing processing logic ...
        
        // Record guard evaluations
        for guard in &transition.guards {
            let guard_start = Instant::now();
            let passed = self.check_guard(guard, &session).await?;
            guards_evaluated.push(GuardResult {
                guard: guard.clone(),
                passed,
                evaluation_time_us: guard_start.elapsed().as_micros() as u64,
            });
        }
        
        // Record action executions
        for action in &transition.actions {
            let action_start = Instant::now();
            match self.execute_action(action, &mut session).await {
                Ok(_) => {
                    actions_executed.push(ActionRecord {
                        action: action.clone(),
                        success: true,
                        execution_time_us: action_start.elapsed().as_micros() as u64,
                        error: None,
                    });
                }
                Err(e) => {
                    let error_msg = e.to_string();
                    errors.push(error_msg.clone());
                    actions_executed.push(ActionRecord {
                        action: action.clone(),
                        success: false,
                        execution_time_us: action_start.elapsed().as_micros() as u64,
                        error: Some(error_msg),
                    });
                }
            }
        }
        
        // Create history record
        let record = TransitionRecord {
            timestamp: Instant::now(),
            sequence: 0, // Will be set by history
            from_state: old_state,
            event: event.clone(),
            to_state: transition.next_state,
            guards_evaluated,
            actions_executed: actions_executed.clone(),
            events_published: transition.publish_events.clone(),
            duration_ms: start_time.elapsed().as_millis() as u64,
            errors,
        };
        
        // Record in session history
        session.record_transition(record);
        
        // ... rest of existing logic ...
    }
}
```

### 1.2 Debugging Inspection Methods

#### Session Inspection API
```rust
// File: src/session_store/inspection.rs

use super::*;

/// Detailed inspection of a session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionInspection {
    /// Current session state
    pub current_state: Option<SessionState>,
    
    /// Recent transition history
    pub recent_transitions: Vec<TransitionRecord>,
    
    /// Possible next transitions
    pub possible_transitions: Vec<PossibleTransition>,
    
    /// Time in current state
    pub time_in_state: Duration,
    
    /// Session age
    pub session_age: Duration,
    
    /// Health status
    pub health: SessionHealth,
    
    /// Resource usage
    pub resources: ResourceUsage,
}

/// A transition that could be taken from current state
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PossibleTransition {
    pub event: EventType,
    pub guards: Vec<Guard>,
    pub next_state: Option<CallState>,
    pub description: String,
}

/// Session health status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SessionHealth {
    Healthy,
    Stale { idle_time: Duration },
    Stuck { state: CallState, duration: Duration },
    ErrorProne { error_rate: f32 },
}

/// Resource usage for the session
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceUsage {
    pub memory_bytes: usize,
    pub history_entries: usize,
    pub active_timers: usize,
    pub pending_events: usize,
}

impl SessionStore {
    /// Inspect a session in detail
    pub async fn inspect_session(&self, session_id: &SessionId) -> SessionInspection {
        let state = self.get_session(session_id).await.ok();
        
        let (recent_transitions, session_age, time_in_state) = if let Some(ref s) = state {
            let history = s.history.as_ref().map(|h| h.get_recent(10)).unwrap_or_default();
            let age = s.history.as_ref()
                .map(|h| h.session_created.elapsed())
                .unwrap_or_default();
            let time = s.entered_state_at.elapsed();
            (history, age, time)
        } else {
            (vec![], Duration::default(), Duration::default())
        };
        
        let possible_transitions = if let Some(ref s) = state {
            self.get_possible_transitions(s)
        } else {
            vec![]
        };
        
        let health = self.assess_health(&state, &recent_transitions, time_in_state);
        let resources = self.calculate_resources(&state);
        
        SessionInspection {
            current_state: state,
            recent_transitions,
            possible_transitions,
            time_in_state,
            session_age,
            health,
            resources,
        }
    }
    
    /// Get all valid transitions from current state
    pub fn get_possible_transitions(&self, state: &SessionState) -> Vec<PossibleTransition> {
        let mut transitions = Vec::new();
        
        // Check all possible events for this role and state
        let events = Self::all_possible_events();
        
        for event in events {
            let key = StateKey {
                role: state.role,
                state: state.call_state,
                event: event.clone(),
            };
            
            if let Some(transition) = MASTER_TABLE.get_transition(&key) {
                transitions.push(PossibleTransition {
                    event,
                    guards: transition.guards.clone(),
                    next_state: transition.next_state,
                    description: Self::describe_transition(&transition),
                });
            }
        }
        
        transitions
    }
    
    /// Get all sessions matching a predicate
    pub async fn find_sessions<F>(&self, predicate: F) -> Vec<SessionId>
    where
        F: Fn(&SessionState) -> bool,
    {
        let sessions = self.sessions.read().await;
        sessions
            .iter()
            .filter(|(_, state)| predicate(state))
            .map(|(id, _)| id.clone())
            .collect()
    }
    
    /// Get sessions in a specific state
    pub async fn get_sessions_in_state(&self, state: CallState) -> Vec<SessionId> {
        self.find_sessions(|s| s.call_state == state).await
    }
    
    /// Get stale sessions
    pub async fn get_stale_sessions(&self, max_idle: Duration) -> Vec<SessionId> {
        self.find_sessions(|s| {
            s.history.as_ref()
                .map(|h| h.last_activity.elapsed() > max_idle)
                .unwrap_or(false)
        }).await
    }
    
    fn assess_health(
        &self,
        state: &Option<SessionState>,
        recent: &[TransitionRecord],
        time_in_state: Duration,
    ) -> SessionHealth {
        if let Some(s) = state {
            // Check if stuck in a state too long
            if time_in_state > Duration::from_secs(300) {
                return SessionHealth::Stuck {
                    state: s.call_state,
                    duration: time_in_state,
                };
            }
            
            // Check error rate
            let error_count = recent.iter().filter(|t| !t.errors.is_empty()).count();
            if error_count > 0 && recent.len() > 0 {
                let error_rate = error_count as f32 / recent.len() as f32;
                if error_rate > 0.3 {
                    return SessionHealth::ErrorProne { error_rate };
                }
            }
            
            // Check if stale
            if let Some(history) = &s.history {
                let idle = history.last_activity.elapsed();
                if idle > Duration::from_secs(60) {
                    return SessionHealth::Stale { idle_time: idle };
                }
            }
        }
        
        SessionHealth::Healthy
    }
    
    fn calculate_resources(&self, state: &Option<SessionState>) -> ResourceUsage {
        if let Some(s) = state {
            let memory = std::mem::size_of_val(s);
            let history_entries = s.history.as_ref()
                .map(|h| h.transitions.len())
                .unwrap_or(0);
            
            ResourceUsage {
                memory_bytes: memory,
                history_entries,
                active_timers: 0, // TODO: Track active timers
                pending_events: 0, // TODO: Track pending events
            }
        } else {
            ResourceUsage {
                memory_bytes: 0,
                history_entries: 0,
                active_timers: 0,
                pending_events: 0,
            }
        }
    }
    
    fn all_possible_events() -> Vec<EventType> {
        vec![
            EventType::MakeCall { target: String::new() },
            EventType::AcceptCall,
            EventType::RejectCall { reason: String::new() },
            EventType::HangupCall,
            EventType::HoldCall,
            EventType::ResumeCall,
            EventType::Dialog180Ringing,
            EventType::Dialog200OK,
            EventType::DialogBYE,
            EventType::DialogCANCEL,
            EventType::MediaEvent("media_ready".to_string()),
            // Add more as needed
        ]
    }
    
    fn describe_transition(transition: &Transition) -> String {
        format!(
            "{} guards, {} actions, next: {:?}",
            transition.guards.len(),
            transition.actions.len(),
            transition.next_state
        )
    }
}
```

### 1.3 Debug Export & Visualization
```rust
// File: src/session_store/export.rs

impl SessionStore {
    /// Export all session data for debugging
    pub async fn export_debug_dump(&self) -> DebugDump {
        let sessions = self.sessions.read().await;
        
        DebugDump {
            timestamp: Instant::now(),
            total_sessions: sessions.len(),
            sessions_by_state: self.count_by_state(&sessions),
            all_sessions: sessions.values().cloned().collect(),
            table_stats: self.get_table_stats(),
        }
    }
    
    /// Export session history as JSON
    pub async fn export_session_history(&self, session_id: &SessionId) -> Result<String> {
        let session = self.get_session(session_id).await?;
        Ok(session.history
            .map(|h| h.export_json())
            .unwrap_or_else(|| "{}".to_string()))
    }
    
    /// Generate Graphviz DOT for state transitions
    pub fn export_state_graph(&self, role: Role) -> String {
        let mut dot = String::from("digraph StateMachine {\n");
        dot.push_str("  rankdir=LR;\n");
        
        // Add states
        for state in Self::all_states() {
            let shape = match state {
                CallState::Idle => "doublecircle",
                CallState::Terminated | CallState::Failed(_) => "doubleoctagon",
                _ => "circle",
            };
            dot.push_str(&format!("  {:?} [shape={}];\n", state, shape));
        }
        
        // Add transitions
        for state in Self::all_states() {
            for event in Self::all_possible_events() {
                let key = StateKey { role, state, event: event.clone() };
                if let Some(transition) = MASTER_TABLE.get_transition(&key) {
                    if let Some(next) = transition.next_state {
                        dot.push_str(&format!(
                            "  {:?} -> {:?} [label=\"{:?}\"];\n",
                            state, next, event
                        ));
                    }
                }
            }
        }
        
        dot.push_str("}\n");
        dot
    }
}

#[derive(Debug, Serialize, Deserialize)]
pub struct DebugDump {
    pub timestamp: Instant,
    pub total_sessions: usize,
    pub sessions_by_state: HashMap<CallState, usize>,
    pub all_sessions: Vec<SessionState>,
    pub table_stats: TableStats,
}

#[derive(Debug, Serialize, Deserialize)]
pub struct TableStats {
    pub total_transitions: usize,
    pub transitions_by_role: HashMap<Role, usize>,
    pub transitions_by_state: HashMap<CallState, usize>,
}
```

## 2. Session Cleanup & Resource Management

### 2.1 Automatic Cleanup System

#### Cleanup Configuration
```rust
// File: src/session_store/cleanup.rs

use std::time::Duration;
use tokio::time::interval;

/// Configuration for automatic cleanup
#[derive(Debug, Clone)]
pub struct CleanupConfig {
    /// How often to run cleanup
    pub interval: Duration,  // Default: 60 seconds
    
    /// TTL for terminated sessions
    pub terminated_ttl: Duration,  // Default: 5 minutes
    
    /// TTL for failed sessions
    pub failed_ttl: Duration,  // Default: 10 minutes
    
    /// Maximum idle time before cleanup
    pub max_idle_time: Duration,  // Default: 1 hour
    
    /// Maximum session age
    pub max_session_age: Duration,  // Default: 24 hours
    
    /// Enable automatic cleanup
    pub enabled: bool,  // Default: true
    
    /// Maximum memory usage before aggressive cleanup
    pub max_memory_bytes: Option<usize>,  // Default: None (unlimited)
}

impl Default for CleanupConfig {
    fn default() -> Self {
        Self {
            interval: Duration::from_secs(60),
            terminated_ttl: Duration::from_secs(300),
            failed_ttl: Duration::from_secs(600),
            max_idle_time: Duration::from_secs(3600),
            max_session_age: Duration::from_secs(86400),
            enabled: true,
            max_memory_bytes: None,
        }
    }
}

/// Statistics from cleanup run
#[derive(Debug, Clone, Default)]
pub struct CleanupStats {
    pub sessions_examined: usize,
    pub sessions_removed: usize,
    pub terminated_removed: usize,
    pub failed_removed: usize,
    pub idle_removed: usize,
    pub aged_removed: usize,
    pub memory_freed_bytes: usize,
    pub cleanup_duration_ms: u64,
}

impl SessionStore {
    /// Start automatic cleanup task
    pub fn start_cleanup_task(self: Arc<Self>, config: CleanupConfig) {
        if !config.enabled {
            return;
        }
        
        tokio::spawn(async move {
            let mut cleanup_interval = interval(config.interval);
            
            loop {
                cleanup_interval.tick().await;
                
                match self.cleanup_stale_sessions(&config).await {
                    Ok(stats) => {
                        if stats.sessions_removed > 0 {
                            tracing::info!(
                                "Cleanup removed {} sessions (freed {} bytes)",
                                stats.sessions_removed,
                                stats.memory_freed_bytes
                            );
                        }
                    }
                    Err(e) => {
                        tracing::error!("Cleanup failed: {}", e);
                    }
                }
            }
        });
    }
    
    /// Perform cleanup of stale sessions
    pub async fn cleanup_stale_sessions(&self, config: &CleanupConfig) -> Result<CleanupStats> {
        let start = Instant::now();
        let mut stats = CleanupStats::default();
        
        let now = Instant::now();
        let mut sessions_to_remove = Vec::new();
        
        // Phase 1: Identify sessions to remove
        {
            let sessions = self.sessions.read().await;
            stats.sessions_examined = sessions.len();
            
            for (id, state) in sessions.iter() {
                let should_remove = self.should_remove_session(state, &config, now);
                
                if should_remove.0 {
                    sessions_to_remove.push((id.clone(), should_remove.1));
                    stats.memory_freed_bytes += std::mem::size_of_val(state);
                    
                    match should_remove.1 {
                        RemovalReason::Terminated => stats.terminated_removed += 1,
                        RemovalReason::Failed => stats.failed_removed += 1,
                        RemovalReason::Idle => stats.idle_removed += 1,
                        RemovalReason::Aged => stats.aged_removed += 1,
                        RemovalReason::MemoryPressure => {},
                    }
                }
            }
        }
        
        // Phase 2: Remove identified sessions
        for (session_id, reason) in sessions_to_remove {
            self.remove_session(&session_id, reason).await?;
            stats.sessions_removed += 1;
        }
        
        // Phase 3: Check memory pressure
        if let Some(max_memory) = config.max_memory_bytes {
            if self.estimate_memory_usage().await > max_memory {
                // Aggressive cleanup - remove oldest idle sessions
                let additional = self.cleanup_for_memory(max_memory).await?;
                stats.sessions_removed += additional;
            }
        }
        
        stats.cleanup_duration_ms = start.elapsed().as_millis() as u64;
        Ok(stats)
    }
    
    fn should_remove_session(
        &self,
        state: &SessionState,
        config: &CleanupConfig,
        now: Instant,
    ) -> (bool, RemovalReason) {
        let state_age = now.duration_since(state.entered_state_at);
        
        // Check terminated sessions
        if state.call_state == CallState::Terminated {
            if state_age > config.terminated_ttl {
                return (true, RemovalReason::Terminated);
            }
        }
        
        // Check failed sessions
        if matches!(state.call_state, CallState::Failed(_)) {
            if state_age > config.failed_ttl {
                return (true, RemovalReason::Failed);
            }
        }
        
        // Check idle time
        if let Some(history) = &state.history {
            let idle_time = now.duration_since(history.last_activity);
            if idle_time > config.max_idle_time {
                return (true, RemovalReason::Idle);
            }
            
            // Check total age
            let age = now.duration_since(history.session_created);
            if age > config.max_session_age {
                return (true, RemovalReason::Aged);
            }
        }
        
        (false, RemovalReason::None)
    }
    
    async fn remove_session(&self, session_id: &SessionId, reason: RemovalReason) -> Result<()> {
        tracing::debug!("Removing session {} (reason: {:?})", session_id, reason);
        
        // Clean up related resources
        self.cleanup_session_resources(session_id).await?;
        
        // Remove from all indexes
        self.sessions.write().await.remove(session_id);
        self.by_dialog.write().await.retain(|_, v| v != session_id);
        self.by_call_id.write().await.retain(|_, v| v != session_id);
        self.by_media_id.write().await.retain(|_, v| v != session_id);
        
        Ok(())
    }
    
    async fn cleanup_session_resources(&self, session_id: &SessionId) -> Result<()> {
        // Clean up any associated resources
        // This would integrate with dialog-core and media-core cleanup
        
        // TODO: Call dialog adapter cleanup
        // TODO: Call media adapter cleanup
        // TODO: Cancel any pending timers
        // TODO: Clear any message queues
        
        Ok(())
    }
    
    async fn estimate_memory_usage(&self) -> usize {
        let sessions = self.sessions.read().await;
        sessions.values().map(|s| std::mem::size_of_val(s)).sum()
    }
    
    async fn cleanup_for_memory(&self, target_bytes: usize) -> Result<usize> {
        // Remove oldest idle sessions until under memory target
        let mut removed = 0;
        
        // Get sessions sorted by last activity
        let mut idle_sessions: Vec<_> = {
            let sessions = self.sessions.read().await;
            sessions
                .iter()
                .filter_map(|(id, state)| {
                    state.history.as_ref().map(|h| (id.clone(), h.last_activity))
                })
                .collect()
        };
        
        idle_sessions.sort_by_key(|(_, last_activity)| *last_activity);
        
        for (session_id, _) in idle_sessions {
            if self.estimate_memory_usage().await <= target_bytes {
                break;
            }
            
            self.remove_session(&session_id, RemovalReason::MemoryPressure).await?;
            removed += 1;
        }
        
        Ok(removed)
    }
}

#[derive(Debug, Clone, Copy)]
enum RemovalReason {
    None,
    Terminated,
    Failed,
    Idle,
    Aged,
    MemoryPressure,
}
```

### 2.2 Resource Limits & Monitoring
```rust
// File: src/session_store/limits.rs

/// Resource limits for the session store
#[derive(Debug, Clone)]
pub struct ResourceLimits {
    /// Maximum number of concurrent sessions
    pub max_sessions: Option<usize>,  // Default: None (unlimited)
    
    /// Maximum memory per session
    pub max_memory_per_session: Option<usize>,  // Default: 10MB
    
    /// Maximum history entries per session
    pub max_history_per_session: usize,  // Default: 100
    
    /// Rate limit for new sessions
    pub max_sessions_per_second: Option<f64>,  // Default: None
}

impl SessionStore {
    /// Check if we can create a new session
    pub async fn can_create_session(&self) -> Result<()> {
        if let Some(max) = self.limits.max_sessions {
            let count = self.sessions.read().await.len();
            if count >= max {
                return Err(SessionError::ResourceExhausted(
                    format!("Maximum sessions ({}) reached", max)
                ));
            }
        }
        
        // Check rate limit
        if let Some(rate) = self.limits.max_sessions_per_second {
            // TODO: Implement rate limiting
        }
        
        Ok(())
    }
    
    /// Get current resource usage
    pub async fn get_resource_usage(&self) -> ResourceUsage {
        let sessions = self.sessions.read().await;
        
        ResourceUsage {
            total_sessions: sessions.len(),
            active_sessions: sessions.values()
                .filter(|s| matches!(s.call_state, CallState::Active))
                .count(),
            total_memory_bytes: sessions.values()
                .map(|s| std::mem::size_of_val(s))
                .sum(),
            history_entries_total: sessions.values()
                .filter_map(|s| s.history.as_ref())
                .map(|h| h.transitions.len())
                .sum(),
        }
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ResourceUsage {
    pub total_sessions: usize,
    pub active_sessions: usize,
    pub total_memory_bytes: usize,
    pub history_entries_total: usize,
}
```

## 3. Integration Points

### 3.1 Configuration
```rust
// Add to src/api/config.rs

#[derive(Debug, Clone)]
pub struct SessionCoreConfig {
    // ... existing fields ...
    
    /// History tracking configuration
    pub history: HistoryConfig,
    
    /// Cleanup configuration
    pub cleanup: CleanupConfig,
    
    /// Resource limits
    pub limits: ResourceLimits,
}

impl Default for SessionCoreConfig {
    fn default() -> Self {
        Self {
            // ... existing defaults ...
            history: HistoryConfig::default(),
            cleanup: CleanupConfig::default(),
            limits: ResourceLimits::default(),
        }
    }
}
```

### 3.2 API Endpoints
```rust
// Add to src/api/unified.rs

impl UnifiedCoordinator {
    /// Get detailed session inspection
    pub async fn inspect_session(&self, session_id: &SessionId) -> SessionInspection {
        self.store.inspect_session(session_id).await
    }
    
    /// Get all sessions in a specific state
    pub async fn get_sessions_by_state(&self, state: CallState) -> Vec<SessionId> {
        self.store.get_sessions_in_state(state).await
    }
    
    /// Export session history
    pub async fn export_session_history(&self, session_id: &SessionId) -> Result<String> {
        self.store.export_session_history(session_id).await
    }
    
    /// Export state graph for visualization
    pub fn export_state_graph(&self, role: Role) -> String {
        self.store.export_state_graph(role)
    }
    
    /// Manually trigger cleanup
    pub async fn cleanup_sessions(&self) -> Result<CleanupStats> {
        self.store.cleanup_stale_sessions(&self.config.cleanup).await
    }
    
    /// Get resource usage
    pub async fn get_resource_usage(&self) -> ResourceUsage {
        self.store.get_resource_usage().await
    }
}
```

## 4. Testing Plan

### 4.1 Unit Tests
```rust
// tests/history_test.rs
#[test]
fn test_history_ring_buffer() {
    // Test that history maintains max size
    // Test sequence numbering
    // Test filtering by state/errors
}

#[test]
fn test_cleanup_logic() {
    // Test TTL-based cleanup
    // Test idle cleanup
    // Test memory pressure cleanup
}
```

### 4.2 Integration Tests
```rust
// tests/session_lifecycle_test.rs
#[tokio::test]
async fn test_full_session_with_history() {
    // Create session with history enabled
    // Process multiple events
    // Verify history is recorded correctly
    // Test inspection API
}

#[tokio::test]
async fn test_automatic_cleanup() {
    // Create multiple sessions
    // Let some go idle/terminated
    // Verify cleanup removes them
    // Check resource usage
}
```

### 4.3 Benchmarks
```rust
// benches/history_overhead.rs
#[bench]
fn bench_with_history_vs_without() {
    // Measure overhead of history tracking
}

#[bench]
fn bench_cleanup_performance() {
    // Measure cleanup with 10K sessions
}
```

## 5. Migration Path

### 5.1 Backward Compatibility
- History is **optional** - disabled by default in release builds
- Cleanup is **opt-in** - must explicitly start cleanup task
- All existing APIs remain unchanged
- New features are additive only

### 5.2 Feature Flags
```toml
[features]
default = []
history = []  # Enable history tracking
cleanup = []  # Enable automatic cleanup
debug-api = ["history"]  # Enable debug endpoints
```

## 6. Performance Considerations

### 6.1 Memory Overhead
- **Per session**: +500 bytes to 5KB (depending on history size)
- **Total for 10K sessions**: +5MB to 50MB
- Configurable limits prevent unbounded growth

### 6.2 CPU Overhead
- **History recording**: <1μs per transition
- **Cleanup task**: <10ms per minute for 1K sessions
- **Inspection API**: <1ms per query

### 6.3 Optimizations
- Ring buffer for history (O(1) operations)
- Indexed lookups for cleanup candidates
- Lazy evaluation of expensive computations
- Batch operations for cleanup

## 7. Documentation

### 7.1 User Guide
```markdown
# Session History and Debugging

## Enabling History
```rust
let config = SessionCoreConfig {
    history: HistoryConfig {
        enabled: true,
        max_transitions: 100,
        ..Default::default()
    },
    ..Default::default()
};
```

## Inspecting Sessions
```rust
let inspection = coordinator.inspect_session(&session_id).await;
println!("Current state: {:?}", inspection.current_state);
println!("Recent transitions: {:?}", inspection.recent_transitions);
```

## Automatic Cleanup
```rust
let config = SessionCoreConfig {
    cleanup: CleanupConfig {
        enabled: true,
        interval: Duration::from_secs(60),
        terminated_ttl: Duration::from_secs(300),
        ..Default::default()
    },
    ..Default::default()
};
```
```

### 7.2 Troubleshooting Guide
- How to export and analyze session history
- Common patterns in transition failures
- Using the state graph for visualization
- Tuning cleanup parameters

## 8. Implementation Schedule

### Day 1: Core History Implementation
- [ ] Implement TransitionRecord and SessionHistory
- [ ] Integrate with SessionState
- [ ] Update StateMachine to record history
- [ ] Write unit tests

### Day 2: Inspection and Debugging
- [ ] Implement SessionInspection API
- [ ] Add get_possible_transitions
- [ ] Implement export functions
- [ ] Create visualization tools

### Day 3: Cleanup and Resource Management
- [ ] Implement CleanupConfig and cleanup logic
- [ ] Add automatic cleanup task
- [ ] Implement resource limits
- [ ] Integration testing

## 9. Success Criteria

1. **History Tracking**
   - ✓ All transitions recorded with <1μs overhead
   - ✓ Ring buffer maintains size limits
   - ✓ Can export history as JSON

2. **Debugging**
   - ✓ Can inspect any session's state and history
   - ✓ Can list possible next transitions
   - ✓ Can generate state graph visualization

3. **Cleanup**
   - ✓ Automatic removal of stale sessions
   - ✓ Configurable TTLs per state
   - ✓ Memory-based cleanup under pressure

4. **Performance**
   - ✓ <5% overhead with history enabled
   - ✓ Cleanup completes in <100ms for 10K sessions
   - ✓ No memory leaks in 24-hour test

## 10. Risks and Mitigations

| Risk | Mitigation |
|------|------------|
| History overhead impacts performance | Make configurable, disable in production |
| Cleanup removes active sessions | Conservative TTLs, extensive testing |
| Memory growth unbounded | Hard limits, aggressive cleanup under pressure |
| Breaking existing code | All changes are additive, extensive tests |

## Questions Before Starting

1. **History depth**: Is 50 transitions enough, or do you need more?
2. **Cleanup intervals**: Is 60 seconds appropriate for cleanup checks?
3. **Memory limits**: Should we enforce hard memory limits?
4. **Export formats**: JSON sufficient, or need other formats?
5. **Visualization**: Is Graphviz DOT format acceptable?
6. **Feature flags**: Should these be compile-time or runtime configurable?