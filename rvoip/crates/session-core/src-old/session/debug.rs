//! Session Debugging and Tracing
//! 
//! This module provides comprehensive debugging and tracing capabilities for session-core:
//! 
//! - Session lifecycle tracing with correlation IDs
//! - Detailed session state change logging
//! - Session debugging utilities and inspection
//! - Distributed tracing support
//! - Session correlation for troubleshooting
//! - Performance metrics and timing analysis

use std::collections::HashMap;
use std::sync::Arc;
use std::time::{Duration, SystemTime, Instant};
use dashmap::DashMap;
use tokio::sync::RwLock;
use tracing::{debug, info, warn, error, trace, span, Level, instrument};
use serde::{Serialize, Deserialize};
use uuid::Uuid;

use crate::session::{SessionId, SessionState};
use crate::errors::{Error, ErrorContext};

/// Session correlation ID for distributed tracing
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct SessionCorrelationId(pub Uuid);

impl SessionCorrelationId {
    /// Create a new correlation ID
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
    
    /// Create from string
    pub fn from_string(s: &str) -> Result<Self, Error> {
        match Uuid::parse_str(s) {
            Ok(uuid) => Ok(Self(uuid)),
            Err(e) => Err(Error::config_error("correlation_id", &format!("Invalid correlation ID: {}", e)))
        }
    }
    
    /// Get as string
    pub fn as_string(&self) -> String {
        self.0.to_string()
    }
}

impl std::fmt::Display for SessionCorrelationId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "{}", self.0)
    }
}

impl Default for SessionCorrelationId {
    fn default() -> Self {
        Self::new()
    }
}

/// Session lifecycle event for tracing
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionLifecycleEvent {
    /// Session ID
    pub session_id: SessionId,
    
    /// Correlation ID for distributed tracing
    pub correlation_id: SessionCorrelationId,
    
    /// Event type
    pub event_type: SessionLifecycleEventType,
    
    /// Session state at time of event
    pub session_state: SessionState,
    
    /// Event timestamp
    pub timestamp: SystemTime,
    
    /// Event duration (for timed events)
    pub duration: Option<Duration>,
    
    /// Additional context data
    pub context: HashMap<String, String>,
    
    /// Error information (if applicable)
    pub error: Option<String>,
}

/// Types of session lifecycle events
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SessionLifecycleEventType {
    /// Session created
    SessionCreated {
        direction: String,
        remote_endpoint: Option<String>,
    },
    
    /// Session state changed
    StateChanged {
        from_state: SessionState,
        to_state: SessionState,
    },
    
    /// Dialog associated with session
    DialogAssociated {
        dialog_id: String,
    },
    
    /// Media session created
    MediaSessionCreated {
        media_session_id: String,
    },
    
    /// Media session terminated
    MediaSessionTerminated {
        media_session_id: String,
        reason: String,
    },
    
    /// Session error occurred
    SessionError {
        error_type: String,
        error_message: String,
    },
    
    /// Session operation started
    OperationStarted {
        operation: String,
    },
    
    /// Session operation completed
    OperationCompleted {
        operation: String,
        success: bool,
    },
    
    /// Session terminated
    SessionTerminated {
        reason: String,
        total_duration: Duration,
    },
}

/// Session debugging information
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionDebugInfo {
    /// Session ID
    pub session_id: SessionId,
    
    /// Correlation ID
    pub correlation_id: SessionCorrelationId,
    
    /// Current session state
    pub current_state: SessionState,
    
    /// Session creation time
    pub created_at: SystemTime,
    
    /// Total session duration
    pub total_duration: Duration,
    
    /// State history (last 20 states)
    pub state_history: Vec<(SessionState, SystemTime)>,
    
    /// Associated dialog IDs
    pub dialog_ids: Vec<String>,
    
    /// Associated media session IDs
    pub media_session_ids: Vec<String>,
    
    /// Recent lifecycle events (last 50)
    pub recent_events: Vec<SessionLifecycleEvent>,
    
    /// Session statistics
    pub statistics: SessionStatistics,
    
    /// Current context data
    pub context: HashMap<String, String>,
    
    /// Session health status
    pub health_status: SessionHealthStatus,
}

/// Session statistics for debugging
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SessionStatistics {
    /// Number of state transitions
    pub state_transitions: usize,
    
    /// Number of errors
    pub error_count: usize,
    
    /// Time in each state
    pub time_per_state: HashMap<SessionState, Duration>,
    
    /// Average operation duration
    pub average_operation_duration: Duration,
    
    /// Peak memory usage
    pub peak_memory_usage: usize,
    
    /// Number of dialog associations
    pub dialog_associations: usize,
    
    /// Number of media sessions
    pub media_sessions: usize,
}

/// Session health status
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SessionHealthStatus {
    /// Session is healthy
    Healthy,
    
    /// Session has warnings
    Warning {
        warnings: Vec<String>,
    },
    
    /// Session is unhealthy
    Unhealthy {
        issues: Vec<String>,
    },
    
    /// Session health is unknown
    Unknown,
}

/// Session tracer for lifecycle tracking
pub struct SessionTracer {
    /// Active session traces
    traces: Arc<DashMap<SessionId, SessionTrace>>,
    
    /// Global correlation mapping
    correlations: Arc<DashMap<SessionCorrelationId, SessionId>>,
    
    /// Event history (for debugging)
    event_history: Arc<RwLock<Vec<SessionLifecycleEvent>>>,
    
    /// Maximum events to keep in history
    max_history_events: usize,
    
    /// Performance metrics
    metrics: Arc<RwLock<TracingMetrics>>,
}

/// Internal session trace data
#[derive(Debug)]
struct SessionTrace {
    /// Session ID
    session_id: SessionId,
    
    /// Correlation ID
    correlation_id: SessionCorrelationId,
    
    /// Session creation time
    created_at: SystemTime,
    
    /// Current state
    current_state: SessionState,
    
    /// State history
    state_history: Vec<(SessionState, SystemTime)>,
    
    /// Lifecycle events
    events: Vec<SessionLifecycleEvent>,
    
    /// Context data
    context: HashMap<String, String>,
    
    /// Active operations (operation_name -> start_time)
    active_operations: HashMap<String, Instant>,
    
    /// Statistics
    statistics: SessionStatistics,
}

/// Tracing metrics
#[derive(Debug, Default)]
pub struct TracingMetrics {
    /// Total sessions traced
    pub total_sessions: u64,
    
    /// Total events recorded
    pub total_events: u64,
    
    /// Events by type
    pub events_by_type: HashMap<String, u64>,
    
    /// Average event processing time
    pub average_event_time: Duration,
}

impl SessionTracer {
    /// Create a new session tracer
    pub fn new(max_history_events: usize) -> Self {
        Self {
            traces: Arc::new(DashMap::new()),
            correlations: Arc::new(DashMap::new()),
            event_history: Arc::new(RwLock::new(Vec::new())),
            max_history_events,
            metrics: Arc::new(RwLock::new(TracingMetrics::default())),
        }
    }
    
    /// Start tracing a session
    #[instrument(level = "debug", skip(self))]
    pub async fn start_session_trace(
        &self,
        session_id: SessionId,
        initial_state: SessionState,
        correlation_id: Option<SessionCorrelationId>,
    ) -> SessionCorrelationId {
        let correlation_id = correlation_id.unwrap_or_else(SessionCorrelationId::new);
        let now = SystemTime::now();
        
        let trace = SessionTrace {
            session_id,
            correlation_id: correlation_id.clone(),
            created_at: now,
            current_state: initial_state,
            state_history: vec![(initial_state, now)],
            events: Vec::new(),
            context: HashMap::new(),
            active_operations: HashMap::new(),
            statistics: SessionStatistics {
                state_transitions: 0,
                error_count: 0,
                time_per_state: HashMap::new(),
                average_operation_duration: Duration::from_secs(0),
                peak_memory_usage: 0,
                dialog_associations: 0,
                media_sessions: 0,
            },
        };
        
        // Store trace
        self.traces.insert(session_id, trace);
        self.correlations.insert(correlation_id.clone(), session_id);
        
        // Record session creation event (clone session_id before moving it)
        let session_id_for_event = session_id;
        self.record_event(session_id_for_event, SessionLifecycleEventType::SessionCreated {
            direction: "unknown".to_string(), // Will be updated when known
            remote_endpoint: None,
        }, None, HashMap::new()).await;
        
        // Update metrics
        let mut metrics = self.metrics.write().await;
        metrics.total_sessions += 1;
        
        info!("🔍 Started session trace for {} with correlation {}", session_id, correlation_id);
        
        correlation_id
    }
    
    /// Record a session lifecycle event
    #[instrument(level = "trace", skip(self, context))]
    pub async fn record_event(
        &self,
        session_id: SessionId,
        event_type: SessionLifecycleEventType,
        duration: Option<Duration>,
        context: HashMap<String, String>,
    ) {
        let session_id_for_log = session_id; // Keep for logging
        let correlation_id = if let Some(mut trace) = self.traces.get_mut(&session_id) {
            let correlation_id = trace.correlation_id.clone();
            
            let event = SessionLifecycleEvent {
                session_id,
                correlation_id: correlation_id.clone(),
                event_type: event_type.clone(),
                session_state: trace.current_state,
                timestamp: SystemTime::now(),
                duration,
                context: context.clone(),
                error: None,
            };
            
            // Add to trace events
            trace.events.push(event.clone());
            
            // Limit event history per session
            if trace.events.len() > 100 {
                trace.events.remove(0);
            }
            
            // Update statistics based on event type
            match &event_type {
                SessionLifecycleEventType::StateChanged { .. } => {
                    trace.statistics.state_transitions += 1;
                },
                SessionLifecycleEventType::SessionError { .. } => {
                    trace.statistics.error_count += 1;
                },
                SessionLifecycleEventType::DialogAssociated { .. } => {
                    trace.statistics.dialog_associations += 1;
                },
                SessionLifecycleEventType::MediaSessionCreated { .. } => {
                    trace.statistics.media_sessions += 1;
                },
                _ => {}
            }
            
            // Add to global event history
            self.add_to_global_history(event.clone()).await;
            
            correlation_id
        } else {
            warn!("Attempted to record event for unknown session {}", session_id_for_log);
            return;
        };
        
        // Update metrics
        let mut metrics = self.metrics.write().await;
        metrics.total_events += 1;
        let event_type_str = format!("{:?}", event_type).split(' ').next().unwrap_or("unknown").to_string();
        *metrics.events_by_type.entry(event_type_str).or_insert(0) += 1;
        
        trace!("📋 Recorded event for session {} ({}): {:?}", session_id_for_log, correlation_id, event_type);
    }
    
    /// Record a session state change
    #[instrument(level = "debug", skip(self))]
    pub async fn record_state_change(
        &self,
        session_id: SessionId,
        from_state: SessionState,
        to_state: SessionState,
    ) {
        let session_id_for_log = session_id;
        if let Some(mut trace) = self.traces.get_mut(&session_id) {
            let now = SystemTime::now();
            
            // Calculate time in previous state
            if let Some((_, last_change)) = trace.state_history.last() {
                if let Ok(duration) = now.duration_since(*last_change) {
                    *trace.statistics.time_per_state.entry(from_state).or_insert(Duration::from_secs(0)) += duration;
                }
            }
            
            // Update current state and history
            trace.current_state = to_state;
            trace.state_history.push((to_state, now));
            
            // Limit state history
            if trace.state_history.len() > 20 {
                trace.state_history.remove(0);
            }
        }
        
        // Record the state change event
        self.record_event(
            session_id,
            SessionLifecycleEventType::StateChanged { from_state, to_state },
            None,
            HashMap::new(),
        ).await;
        
        info!("🔄 Session {} state change: {} → {}", session_id_for_log, from_state, to_state);
    }
    
    /// Start tracking an operation
    #[instrument(level = "debug", skip(self))]
    pub async fn start_operation(&self, session_id: SessionId, operation: &str) {
        let session_id_for_log = session_id;
        if let Some(mut trace) = self.traces.get_mut(&session_id) {
            trace.active_operations.insert(operation.to_string(), Instant::now());
        }
        
        self.record_event(
            session_id,
            SessionLifecycleEventType::OperationStarted {
                operation: operation.to_string(),
            },
            None,
            HashMap::new(),
        ).await;
        
        debug!("🔧 Started operation '{}' for session {}", operation, session_id_for_log);
    }
    
    /// Complete tracking an operation
    #[instrument(level = "debug", skip(self))]
    pub async fn complete_operation(&self, session_id: SessionId, operation: &str, success: bool) {
        let session_id_for_log = session_id;
        let duration = if let Some(mut trace) = self.traces.get_mut(&session_id) {
            if let Some(start_time) = trace.active_operations.remove(operation) {
                let duration = start_time.elapsed();
                
                // Update average operation duration
                let current_avg = trace.statistics.average_operation_duration;
                let operations_count = trace.statistics.state_transitions + 1; // Rough estimate
                trace.statistics.average_operation_duration = 
                    (current_avg * (operations_count - 1) as u32 + duration) / operations_count as u32;
                
                Some(duration)
            } else {
                None
            }
        } else {
            None
        };
        
        self.record_event(
            session_id,
            SessionLifecycleEventType::OperationCompleted {
                operation: operation.to_string(),
                success,
            },
            duration,
            HashMap::new(),
        ).await;
        
        if let Some(duration) = duration {
            debug!("✅ Completed operation '{}' for session {} in {:?} (success: {})", 
                operation, session_id_for_log, duration, success);
        } else {
            debug!("✅ Completed operation '{}' for session {} (success: {})", 
                operation, session_id_for_log, success);
        }
    }
    
    /// Add context data to a session trace
    pub async fn add_context(&self, session_id: SessionId, key: &str, value: &str) {
        if let Some(mut trace) = self.traces.get_mut(&session_id) {
            trace.context.insert(key.to_string(), value.to_string());
        }
        
        trace!("📝 Added context to session {}: {} = {}", session_id, key, value);
    }
    
    /// Record a session error
    #[instrument(level = "warn", skip(self, error))]
    pub async fn record_error(&self, session_id: SessionId, error: &Error) {
        let session_id_for_log = session_id;
        let error_type = format!("{:?}", error).split('(').next().unwrap_or("unknown").to_string();
        let error_message = error.to_string();
        
        // Add error details to context
        let mut context = HashMap::new();
        context.insert("error_category".to_string(), error.category().to_string());
        context.insert("error_severity".to_string(), error.severity().to_string());
        context.insert("retryable".to_string(), error.is_retryable().to_string());
        
        if let Some(session_id_str) = &error.context().session_id {
            context.insert("error_session_id".to_string(), session_id_str.clone());
        }
        
        self.record_event(
            session_id,
            SessionLifecycleEventType::SessionError {
                error_type,
                error_message,
            },
            None,
            context,
        ).await;
        
        warn!("❌ Recorded error for session {}: {}", session_id_for_log, error);
    }
    
    /// Terminate session tracing
    #[instrument(level = "debug", skip(self))]
    pub async fn terminate_session_trace(&self, session_id: SessionId, reason: &str) {
        let session_id_for_log = session_id;
        let total_duration = if let Some(trace) = self.traces.get(&session_id) {
            SystemTime::now().duration_since(trace.created_at).unwrap_or(Duration::from_secs(0))
        } else {
            Duration::from_secs(0)
        };
        
        // Record termination event
        self.record_event(
            session_id,
            SessionLifecycleEventType::SessionTerminated {
                reason: reason.to_string(),
                total_duration,
            },
            Some(total_duration),
            HashMap::new(),
        ).await;
        
        // Remove from active traces (but keep correlation mapping for a while)
        if let Some((_, trace)) = self.traces.remove(&session_id_for_log) {
            info!("🔚 Terminated session trace for {} (duration: {:?}, events: {}, errors: {})", 
                session_id_for_log, total_duration, trace.events.len(), trace.statistics.error_count);
        }
    }
    
    /// Get debug information for a session
    pub async fn get_session_debug_info(&self, session_id: SessionId) -> Option<SessionDebugInfo> {
        let trace = self.traces.get(&session_id)?;
        let now = SystemTime::now();
        let total_duration = now.duration_since(trace.created_at).unwrap_or(Duration::from_secs(0));
        
        // Determine health status
        let health_status = if trace.statistics.error_count > 5 {
            SessionHealthStatus::Unhealthy {
                issues: vec!["High error count".to_string()],
            }
        } else if trace.statistics.error_count > 0 {
            SessionHealthStatus::Warning {
                warnings: vec![format!("{} errors occurred", trace.statistics.error_count)],
            }
        } else {
            SessionHealthStatus::Healthy
        };
        
        Some(SessionDebugInfo {
            session_id,
            correlation_id: trace.correlation_id.clone(),
            current_state: trace.current_state,
            created_at: trace.created_at,
            total_duration,
            state_history: trace.state_history.clone(),
            dialog_ids: Vec::new(), // Would be populated from actual session data
            media_session_ids: Vec::new(), // Would be populated from actual session data
            recent_events: trace.events.iter().rev().take(50).cloned().collect(),
            statistics: trace.statistics.clone(),
            context: trace.context.clone(),
            health_status,
        })
    }
    
    /// Get session by correlation ID
    pub async fn get_session_by_correlation(&self, correlation_id: &SessionCorrelationId) -> Option<SessionId> {
        self.correlations.get(correlation_id).map(|entry| entry.value().clone())
    }
    
    /// Get tracing metrics
    pub async fn get_metrics(&self) -> TracingMetrics {
        self.metrics.read().await.clone()
    }
    
    /// Add event to global history
    async fn add_to_global_history(&self, event: SessionLifecycleEvent) {
        let mut history = self.event_history.write().await;
        history.push(event);
        
        // Limit global history size
        if history.len() > self.max_history_events {
            history.remove(0);
        }
    }
}

impl Clone for TracingMetrics {
    fn clone(&self) -> Self {
        Self {
            total_sessions: self.total_sessions,
            total_events: self.total_events,
            events_by_type: self.events_by_type.clone(),
            average_event_time: self.average_event_time,
        }
    }
}

/// Session debugging utilities
pub struct SessionDebugger;

impl SessionDebugger {
    /// Analyze session debug information for issues
    pub fn analyze_session_health(debug_info: &SessionDebugInfo) -> Vec<String> {
        let mut issues = Vec::new();
        
        // Check for excessive state transitions
        if debug_info.statistics.state_transitions > 20 {
            issues.push(format!("Excessive state transitions: {}", debug_info.statistics.state_transitions));
        }
        
        // Check for high error rate
        if debug_info.statistics.error_count > 5 {
            issues.push(format!("High error count: {}", debug_info.statistics.error_count));
        }
        
        // Check for long duration in non-terminal states
        for (state, duration) in &debug_info.statistics.time_per_state {
            if *state != SessionState::Connected && *state != SessionState::Terminated {
                if duration > &Duration::from_secs(300) { // 5 minutes
                    issues.push(format!("Long duration in state {:?}: {:?}", state, duration));
                }
            }
        }
        
        // Check session age
        if debug_info.total_duration > Duration::from_secs(3600) { // 1 hour
            issues.push(format!("Long-running session: {:?}", debug_info.total_duration));
        }
        
        issues
    }
    
    /// Generate a human-readable session timeline
    pub fn generate_session_timeline(debug_info: &SessionDebugInfo) -> String {
        let mut timeline = String::new();
        timeline.push_str(&format!("Session Timeline for {}\n", debug_info.session_id));
        timeline.push_str(&format!("Correlation ID: {}\n", debug_info.correlation_id));
        timeline.push_str(&format!("Total Duration: {:?}\n", debug_info.total_duration));
        timeline.push_str(&format!("Current State: {:?}\n", debug_info.current_state));
        timeline.push_str(&format!("Health: {:?}\n\n", debug_info.health_status));
        
        timeline.push_str("Recent Events:\n");
        for event in debug_info.recent_events.iter().rev().take(10) {
            let duration_str = if let Some(duration) = event.duration {
                format!(" ({:?})", duration)
            } else {
                String::new()
            };
            
            timeline.push_str(&format!("  {:?} - {:?}{}\n", 
                event.timestamp, event.event_type, duration_str));
        }
        
        timeline.push_str("\nStatistics:\n");
        timeline.push_str(&format!("  State Transitions: {}\n", debug_info.statistics.state_transitions));
        timeline.push_str(&format!("  Errors: {}\n", debug_info.statistics.error_count));
        timeline.push_str(&format!("  Dialog Associations: {}\n", debug_info.statistics.dialog_associations));
        timeline.push_str(&format!("  Media Sessions: {}\n", debug_info.statistics.media_sessions));
        
        timeline
    }
} 