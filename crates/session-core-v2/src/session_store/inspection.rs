//! Session inspection and debugging utilities

use std::time::{Duration, Instant};
use std::collections::HashMap;
use serde::{Serialize, Deserialize};
use crate::state_table::{
    SessionId, Role, EventType, Guard, StateKey, MASTER_TABLE
};
use crate::types::{CallState, FailureReason};
use super::{SessionStore, SessionState};
use super::history::TransitionRecord;

/// Detailed inspection of a session
#[derive(Debug, Clone)]
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

/// Debug dump of all sessions
#[derive(Debug)]
pub struct DebugDump {
    pub timestamp: Instant,
    pub total_sessions: usize,
    pub sessions_by_state: HashMap<String, usize>,
    pub all_sessions: Vec<SessionState>,
    pub table_stats: TableStats,
}

/// Statistics about the state table
#[derive(Debug, Serialize, Deserialize)]
pub struct TableStats {
    pub total_transitions: usize,
    pub transitions_by_role: HashMap<String, usize>,
    pub transitions_by_state: HashMap<String, usize>,
}

impl SessionStore {
    /// Inspect a session in detail
    pub async fn inspect_session(&self, session_id: &SessionId) -> SessionInspection {
        let state = self.get_session(session_id).await.ok();
        
        let (recent_transitions, session_age, time_in_state) = if let Some(ref s) = state {
            let history = s.history.as_ref().map(|h| h.get_recent(10)).unwrap_or_default();
            let age = s.history.as_ref()
                .map(|h| h.session_age())
                .unwrap_or_else(|| s.created_at.elapsed());
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
                    event: event.clone(),
                    guards: transition.guards.clone(),
                    next_state: transition.next_state,
                    description: Self::describe_transition(&transition),
                });
            }
            
            // Also check "Both" role
            if state.role != Role::Both {
                let key_both = StateKey {
                    role: Role::Both,
                    state: state.call_state,
                    event: event.clone(),
                };
                
                if let Some(transition) = MASTER_TABLE.get_transition(&key_both) {
                    if !transitions.iter().any(|t| std::mem::discriminant(&t.event) == std::mem::discriminant(&event)) {
                        transitions.push(PossibleTransition {
                            event: event.clone(),
                            guards: transition.guards.clone(),
                            next_state: transition.next_state,
                            description: Self::describe_transition(&transition),
                        });
                    }
                }
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
                .map(|h| h.is_idle(max_idle))
                .unwrap_or(false)
        }).await
    }
    
    /// Export all session data for debugging
    pub async fn export_debug_dump(&self) -> DebugDump {
        let sessions = self.sessions.read().await;
        
        let mut sessions_by_state = HashMap::new();
        for state in sessions.values() {
            let state_name = format!("{:?}", state.call_state);
            *sessions_by_state.entry(state_name).or_insert(0) += 1;
        }
        
        DebugDump {
            timestamp: Instant::now(),
            total_sessions: sessions.len(),
            sessions_by_state,
            all_sessions: sessions.values().cloned().collect(),
            table_stats: self.get_table_stats(),
        }
    }
    
    /// Export session history as JSON
    pub async fn export_session_history(&self, session_id: &SessionId) -> crate::errors::Result<String> {
        let session = self.get_session(session_id).await?;
        Ok(session.history
            .map(|h| h.export_json())
            .unwrap_or_else(|| "{}".to_string()))
    }
    
    /// Export session history as CSV
    pub async fn export_session_history_csv(&self, session_id: &SessionId) -> crate::errors::Result<String> {
        let session = self.get_session(session_id).await?;
        Ok(session.history
            .map(|h| h.export_csv())
            .unwrap_or_else(|| "sequence,timestamp_ms,from_state,event,to_state,duration_ms,errors\n".to_string()))
    }
    
    /// Generate Graphviz DOT for state transitions
    pub fn export_state_graph(&self, role: Role) -> String {
        let mut dot = String::from("digraph StateMachine {\n");
        dot.push_str("  rankdir=LR;\n");
        dot.push_str("  node [fontname=\"Arial\"];\n");
        dot.push_str("  edge [fontname=\"Arial\"];\n\n");
        
        // Add states with styling
        for state in Self::all_states() {
            let (shape, color) = match state {
                CallState::Idle => ("doublecircle", "green"),
                CallState::Terminated => ("doubleoctagon", "red"),
                CallState::Failed(_) => ("doubleoctagon", "crimson"),
                CallState::Active => ("circle", "blue"),
                _ => ("circle", "black"),
            };
            dot.push_str(&format!(
                "  \"{:?}\" [shape={}, color={}];\n",
                state, shape, color
            ));
        }
        dot.push_str("\n");
        
        // Add transitions
        let mut transition_map: HashMap<(CallState, CallState), Vec<String>> = HashMap::new();
        
        for state in Self::all_states() {
            for event in Self::all_possible_events() {
                let key = StateKey { role, state, event: event.clone() };
                if let Some(transition) = MASTER_TABLE.get_transition(&key) {
                    if let Some(next) = transition.next_state {
                        let event_name = Self::event_short_name(&event);
                        transition_map
                            .entry((state, next))
                            .or_insert_with(Vec::new)
                            .push(event_name);
                    }
                }
                
                // Also check "Both" role
                if role != Role::Both {
                    let key_both = StateKey { role: Role::Both, state, event: event.clone() };
                    if let Some(transition) = MASTER_TABLE.get_transition(&key_both) {
                        if let Some(next) = transition.next_state {
                            let event_name = Self::event_short_name(&event);
                            transition_map
                                .entry((state, next))
                                .or_insert_with(Vec::new)
                                .push(event_name);
                        }
                    }
                }
            }
        }
        
        // Write deduplicated transitions
        for ((from, to), events) in transition_map {
            let label = if events.len() <= 3 {
                events.join("\\n")
            } else {
                format!("{}\\n...({} total)", events[..3].join("\\n"), events.len())
            };
            dot.push_str(&format!(
                "  \"{:?}\" -> \"{:?}\" [label=\"{}\"];\n",
                from, to, label
            ));
        }
        
        dot.push_str("}\n");
        dot
    }
    
    fn assess_health(
        &self,
        state: &Option<SessionState>,
        recent: &[TransitionRecord],
        time_in_state: Duration,
    ) -> SessionHealth {
        if let Some(s) = state {
            // Check if stuck in a state too long (except Active/OnHold which are expected to be long)
            if !matches!(s.call_state, CallState::Active | CallState::OnHold) {
                if time_in_state > Duration::from_secs(300) {
                    return SessionHealth::Stuck {
                        state: s.call_state,
                        duration: time_in_state,
                    };
                }
            }
            
            // Check error rate
            if !recent.is_empty() {
                let error_count = recent.iter().filter(|t| !t.errors.is_empty()).count();
                let error_rate = error_count as f32 / recent.len() as f32;
                if error_rate > 0.3 {
                    return SessionHealth::ErrorProne { error_rate };
                }
            }
            
            // Check if stale
            if let Some(history) = &s.history {
                let idle = history.idle_time();
                if idle > Duration::from_secs(3600) {
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
                .map(|h| h.total_transitions as usize)
                .unwrap_or(0);
            
            ResourceUsage {
                memory_bytes: memory,
                history_entries,
                active_timers: 0, // TODO: Track active timers when implemented
                pending_events: 0, // TODO: Track pending events when implemented
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
    
    fn get_table_stats(&self) -> TableStats {
        let mut total = 0;
        let mut by_role = HashMap::new();
        let mut by_state = HashMap::new();
        
        // Count transitions in the master table
        // This is an approximation since we can't iterate the HashMap directly
        for role in [Role::UAC, Role::UAS, Role::Both] {
            let role_name = format!("{:?}", role);
            let mut role_count = 0;
            
            for state in Self::all_states() {
                let state_name = format!("{:?}", state);
                let mut state_count = 0;
                
                for event in Self::all_possible_events() {
                    let key = StateKey { role, state, event };
                    if MASTER_TABLE.has_transition(&key) {
                        total += 1;
                        role_count += 1;
                        state_count += 1;
                    }
                }
                
                if state_count > 0 {
                    *by_state.entry(state_name).or_insert(0) += state_count;
                }
            }
            
            if role_count > 0 {
                by_role.insert(role_name, role_count);
            }
        }
        
        TableStats {
            total_transitions: total,
            transitions_by_role: by_role,
            transitions_by_state: by_state,
        }
    }
    
    fn all_possible_events() -> Vec<EventType> {
        vec![
            EventType::MakeCall { target: String::new() },
            EventType::IncomingCall { from: String::new(), sdp: None },
            EventType::AcceptCall,
            EventType::RejectCall { reason: String::new() },
            EventType::HangupCall,
            EventType::HoldCall,
            EventType::ResumeCall,
            EventType::Dialog180Ringing,
            EventType::Dialog183SessionProgress,
            EventType::Dialog200OK,
            EventType::DialogACK,
            EventType::DialogBYE,
            EventType::DialogCANCEL,
            EventType::DialogReINVITE,
            EventType::Dialog4xxFailure(400),
            EventType::Dialog5xxFailure(500),
            EventType::Dialog6xxFailure(600),
            EventType::DialogTimeout,
            EventType::DialogTerminated,
            EventType::MediaEvent("media_ready".to_string()),
            EventType::MediaEvent("media_failed".to_string()),
            EventType::CheckConditions,
            EventType::BridgeSessions { other_session: SessionId::new() },
            EventType::UnbridgeSessions,
            EventType::BlindTransfer { target: String::new() },
            EventType::TransferComplete,
            EventType::TransferFailed,
        ]
    }
    
    fn all_states() -> Vec<CallState> {
        vec![
            CallState::Idle,
            CallState::Initiating,
            CallState::Ringing,
            CallState::EarlyMedia,
            CallState::Active,
            CallState::OnHold,
            CallState::Resuming,
            CallState::Bridged,
            CallState::Transferring,
            CallState::Terminating,
            CallState::Terminated,
            CallState::Failed(FailureReason::Other),
        ]
    }
    
    fn describe_transition(transition: &crate::state_table::Transition) -> String {
        format!(
            "{} guards, {} actions, next: {:?}",
            transition.guards.len(),
            transition.actions.len(),
            transition.next_state
        )
    }
    
    fn event_short_name(event: &EventType) -> String {
        match event {
            EventType::MakeCall { .. } => "MakeCall".to_string(),
            EventType::IncomingCall { .. } => "IncomingCall".to_string(),
            EventType::AcceptCall => "Accept".to_string(),
            EventType::RejectCall { .. } => "Reject".to_string(),
            EventType::HangupCall => "Hangup".to_string(),
            EventType::HoldCall => "Hold".to_string(),
            EventType::ResumeCall => "Resume".to_string(),
            EventType::Dialog180Ringing => "180".to_string(),
            EventType::Dialog183SessionProgress => "183".to_string(),
            EventType::Dialog200OK => "200OK".to_string(),
            EventType::DialogACK => "ACK".to_string(),
            EventType::DialogBYE => "BYE".to_string(),
            EventType::DialogCANCEL => "CANCEL".to_string(),
            EventType::DialogReINVITE => "reINVITE".to_string(),
            EventType::Dialog4xxFailure(_) => "4xx".to_string(),
            EventType::Dialog5xxFailure(_) => "5xx".to_string(),
            EventType::Dialog6xxFailure(_) => "6xx".to_string(),
            EventType::DialogTimeout => "Timeout".to_string(),
            EventType::MediaEvent(s) => s.clone(),
            _ => format!("{:?}", event).split("::").last().unwrap_or("?").to_string(),
        }
    }
}