//! Cross-Session Event Propagation
//! 
//! This module provides sophisticated event propagation across related sessions:
//! 
//! - Event broadcasting within session groups
//! - Selective event propagation based on rules
//! - Cross-session state synchronization
//! - Event filtering and transformation
//! - Cascading event processing

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::SystemTime;
use dashmap::DashMap;
use tokio::sync::{broadcast, RwLock};
use tracing::{debug, info, warn, error};
use serde::{Serialize, Deserialize};
use uuid::Uuid;

use crate::session::{SessionId, SessionState};
use crate::errors::{Error, ErrorContext};

/// Types of session coordination events
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SessionCoordinationEvent {
    /// Session state changed
    StateChanged {
        session_id: SessionId,
        old_state: SessionState,
        new_state: SessionState,
        timestamp: SystemTime,
    },
    
    /// Session media state changed
    MediaStateChanged {
        session_id: SessionId,
        media_state: String,
        timestamp: SystemTime,
    },
    
    /// Session joined a group
    SessionJoinedGroup {
        session_id: SessionId,
        group_id: String,
        role: String,
        timestamp: SystemTime,
    },
    
    /// Session left a group
    SessionLeftGroup {
        session_id: SessionId,
        group_id: String,
        reason: String,
        timestamp: SystemTime,
    },
    
    /// Session error occurred
    SessionError {
        session_id: SessionId,
        error_type: String,
        error_message: String,
        timestamp: SystemTime,
    },
    
    /// Custom coordination event
    Custom {
        event_type: String,
        session_id: SessionId,
        data: HashMap<String, String>,
        timestamp: SystemTime,
    },
}

/// Rules for event propagation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum PropagationRule {
    /// Propagate to all sessions in the same group
    ToGroupMembers {
        group_id: Option<String>, // None = all groups
    },
    
    /// Propagate to dependent sessions
    ToDependentSessions,
    
    /// Propagate to sessions with specific roles
    ToSessionsWithRole {
        role: String,
    },
    
    /// Propagate to specific sessions
    ToSpecificSessions {
        session_ids: Vec<SessionId>,
    },
    
    /// Custom propagation rule
    Custom {
        rule_name: String,
        parameters: HashMap<String, String>,
    },
}

/// Event filter for selective propagation
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EventFilter {
    /// Event types to include (empty = all)
    pub include_events: HashSet<String>,
    
    /// Event types to exclude
    pub exclude_events: HashSet<String>,
    
    /// Session IDs to include (empty = all)
    pub include_sessions: HashSet<SessionId>,
    
    /// Session IDs to exclude
    pub exclude_sessions: HashSet<SessionId>,
    
    /// Minimum event priority
    pub min_priority: u32,
    
    /// Custom filter criteria
    pub custom_criteria: HashMap<String, String>,
}

impl Default for EventFilter {
    fn default() -> Self {
        Self {
            include_events: HashSet::new(),
            exclude_events: HashSet::new(),
            include_sessions: HashSet::new(),
            exclude_sessions: HashSet::new(),
            min_priority: 0,
            custom_criteria: HashMap::new(),
        }
    }
}

impl EventFilter {
    /// Check if an event passes the filter
    pub fn matches(&self, event: &SessionCoordinationEvent, priority: u32) -> bool {
        // Check priority
        if priority < self.min_priority {
            return false;
        }
        
        let event_type = match event {
            SessionCoordinationEvent::StateChanged { .. } => "StateChanged",
            SessionCoordinationEvent::MediaStateChanged { .. } => "MediaStateChanged",
            SessionCoordinationEvent::SessionJoinedGroup { .. } => "SessionJoinedGroup",
            SessionCoordinationEvent::SessionLeftGroup { .. } => "SessionLeftGroup",
            SessionCoordinationEvent::SessionError { .. } => "SessionError",
            SessionCoordinationEvent::Custom { event_type, .. } => event_type,
        };
        
        let session_id = match event {
            SessionCoordinationEvent::StateChanged { session_id, .. } => *session_id,
            SessionCoordinationEvent::MediaStateChanged { session_id, .. } => *session_id,
            SessionCoordinationEvent::SessionJoinedGroup { session_id, .. } => *session_id,
            SessionCoordinationEvent::SessionLeftGroup { session_id, .. } => *session_id,
            SessionCoordinationEvent::SessionError { session_id, .. } => *session_id,
            SessionCoordinationEvent::Custom { session_id, .. } => *session_id,
        };
        
        // Check event type inclusion/exclusion
        if !self.include_events.is_empty() && !self.include_events.contains(event_type) {
            return false;
        }
        
        if self.exclude_events.contains(event_type) {
            return false;
        }
        
        // Check session inclusion/exclusion
        if !self.include_sessions.is_empty() && !self.include_sessions.contains(&session_id) {
            return false;
        }
        
        if self.exclude_sessions.contains(&session_id) {
            return false;
        }
        
        true
    }
}

/// Event propagation configuration
#[derive(Debug, Clone)]
pub struct PropagationConfig {
    /// Maximum events to buffer
    pub max_buffer_size: usize,
    
    /// Whether to track propagation metrics
    pub track_metrics: bool,
    
    /// Default event priority
    pub default_priority: u32,
    
    /// Whether to prevent infinite propagation loops
    pub prevent_loops: bool,
    
    /// Maximum propagation depth
    pub max_propagation_depth: usize,
}

impl Default for PropagationConfig {
    fn default() -> Self {
        Self {
            max_buffer_size: 1000,
            track_metrics: true,
            default_priority: 50,
            prevent_loops: true,
            max_propagation_depth: 5,
        }
    }
}

/// Metrics for event propagation
#[derive(Debug, Clone, Default, Serialize, Deserialize)]
pub struct PropagationMetrics {
    /// Total events propagated
    pub total_events_propagated: u64,
    
    /// Events by type
    pub events_by_type: HashMap<String, u64>,
    
    /// Propagation by rule type
    pub propagations_by_rule: HashMap<String, u64>,
    
    /// Filtered events (didn't pass filter)
    pub filtered_events: u64,
    
    /// Loop prevention activations
    pub loop_preventions: u64,
    
    /// Average propagation latency
    pub average_propagation_latency: std::time::Duration,
}

/// Cross-session event propagator
pub struct CrossSessionEventPropagator {
    /// Event broadcasters by group/context
    broadcasters: Arc<DashMap<String, broadcast::Sender<SessionCoordinationEvent>>>,
    
    /// Propagation rules by context
    propagation_rules: Arc<DashMap<String, Vec<PropagationRule>>>,
    
    /// Event filters by context
    event_filters: Arc<DashMap<String, EventFilter>>,
    
    /// Session to context mappings
    session_contexts: Arc<DashMap<SessionId, HashSet<String>>>,
    
    /// Propagation metrics
    metrics: Arc<RwLock<PropagationMetrics>>,
    
    /// Event loop detection (event_id -> seen_contexts)
    loop_detection: Arc<DashMap<String, HashSet<String>>>,
    
    /// Configuration
    config: PropagationConfig,
}

impl CrossSessionEventPropagator {
    /// Create a new event propagator
    pub fn new(config: PropagationConfig) -> Self {
        Self {
            broadcasters: Arc::new(DashMap::new()),
            propagation_rules: Arc::new(DashMap::new()),
            event_filters: Arc::new(DashMap::new()),
            session_contexts: Arc::new(DashMap::new()),
            metrics: Arc::new(RwLock::new(PropagationMetrics::default())),
            loop_detection: Arc::new(DashMap::new()),
            config,
        }
    }
    
    /// Create a propagation context (group, sequence, etc.)
    pub async fn create_context(&self, context_id: String) -> Result<(), Error> {
        let (tx, _) = broadcast::channel(self.config.max_buffer_size);
        self.broadcasters.insert(context_id.clone(), tx);
        
        info!("✅ Created propagation context {}", context_id);
        Ok(())
    }
    
    /// Add a session to a propagation context
    pub async fn add_session_to_context(
        &self,
        session_id: SessionId,
        context_id: String,
    ) -> Result<(), Error> {
        self.session_contexts
            .entry(session_id)
            .or_insert_with(HashSet::new)
            .insert(context_id.clone());
        
        info!("✅ Added session {} to propagation context {}", session_id, context_id);
        Ok(())
    }
    
    /// Remove a session from a propagation context
    pub async fn remove_session_from_context(
        &self,
        session_id: SessionId,
        context_id: &str,
    ) -> Result<(), Error> {
        if let Some(mut contexts) = self.session_contexts.get_mut(&session_id) {
            contexts.remove(context_id);
            if contexts.is_empty() {
                drop(contexts);
                self.session_contexts.remove(&session_id);
            }
        }
        
        info!("🗑️ Removed session {} from propagation context {}", session_id, context_id);
        Ok(())
    }
    
    /// Set propagation rules for a context
    pub async fn set_propagation_rules(
        &self,
        context_id: String,
        rules: Vec<PropagationRule>,
    ) -> Result<(), Error> {
        self.propagation_rules.insert(context_id.clone(), rules);
        
        info!("⚙️ Set propagation rules for context {}", context_id);
        Ok(())
    }
    
    /// Set event filter for a context
    pub async fn set_event_filter(
        &self,
        context_id: String,
        filter: EventFilter,
    ) -> Result<(), Error> {
        self.event_filters.insert(context_id.clone(), filter);
        
        info!("🔍 Set event filter for context {}", context_id);
        Ok(())
    }
    
    /// Propagate an event
    pub async fn propagate_event(
        &self,
        event: SessionCoordinationEvent,
        context_id: Option<String>,
        priority: Option<u32>,
    ) -> Result<usize, Error> {
        let start_time = std::time::Instant::now();
        let priority = priority.unwrap_or(self.config.default_priority);
        let event_id = Uuid::new_v4().to_string();
        
        let mut propagation_count = 0;
        
        // Determine contexts to propagate to
        let contexts = if let Some(context) = context_id {
            vec![context]
        } else {
            // Get all contexts for the source session
            let source_session = match &event {
                SessionCoordinationEvent::StateChanged { session_id, .. } => *session_id,
                SessionCoordinationEvent::MediaStateChanged { session_id, .. } => *session_id,
                SessionCoordinationEvent::SessionJoinedGroup { session_id, .. } => *session_id,
                SessionCoordinationEvent::SessionLeftGroup { session_id, .. } => *session_id,
                SessionCoordinationEvent::SessionError { session_id, .. } => *session_id,
                SessionCoordinationEvent::Custom { session_id, .. } => *session_id,
            };
            
            if let Some(session_contexts) = self.session_contexts.get(&source_session) {
                session_contexts.iter().cloned().collect()
            } else {
                Vec::new()
            }
        };
        
        // Propagate to each context
        for context in &contexts {
            // Check for loops
            if self.config.prevent_loops {
                let mut loop_contexts = self.loop_detection.entry(event_id.clone()).or_insert_with(HashSet::new);
                if loop_contexts.contains(context) {
                    if self.config.track_metrics {
                        let mut metrics = self.metrics.write().await;
                        metrics.loop_preventions += 1;
                    }
                    continue;
                }
                loop_contexts.insert(context.clone());
            }
            
            // Apply event filter
            if let Some(filter) = self.event_filters.get(context) {
                if !filter.matches(&event, priority) {
                    if self.config.track_metrics {
                        let mut metrics = self.metrics.write().await;
                        metrics.filtered_events += 1;
                    }
                    continue;
                }
            }
            
            // Broadcast event
            if let Some(broadcaster) = self.broadcasters.get(context) {
                match broadcaster.send(event.clone()) {
                    Ok(receiver_count) => {
                        propagation_count += receiver_count;
                        debug!("📡 Propagated event to {} receivers in context {}", receiver_count, context);
                    },
                    Err(_) => {
                        debug!("No active receivers for context {}", context);
                    }
                }
            }
        }
        
        // Update metrics
        if self.config.track_metrics {
            let mut metrics = self.metrics.write().await;
            metrics.total_events_propagated += 1;
            
            let event_type = match &event {
                SessionCoordinationEvent::StateChanged { .. } => "StateChanged",
                SessionCoordinationEvent::MediaStateChanged { .. } => "MediaStateChanged",
                SessionCoordinationEvent::SessionJoinedGroup { .. } => "SessionJoinedGroup",
                SessionCoordinationEvent::SessionLeftGroup { .. } => "SessionLeftGroup",
                SessionCoordinationEvent::SessionError { .. } => "SessionError",
                SessionCoordinationEvent::Custom { event_type, .. } => event_type,
            };
            
            *metrics.events_by_type.entry(event_type.to_string()).or_insert(0) += 1;
            
            // Update average latency
            let latency = start_time.elapsed();
            metrics.average_propagation_latency = 
                (metrics.average_propagation_latency * (metrics.total_events_propagated - 1) as u32 + latency) 
                / metrics.total_events_propagated as u32;
        }
        
        // Clean up loop detection after some time
        if self.config.prevent_loops {
            // In a real implementation, you'd want to clean this up periodically
            // For now, we'll just let it grow (should be cleaned up on context removal)
        }
        
        info!("📡 Propagated event to {} receivers across {} contexts", 
            propagation_count, contexts.len());
        
        Ok(propagation_count)
    }
    
    /// Subscribe to events in a context
    pub async fn subscribe_to_context(
        &self,
        context_id: &str,
    ) -> Result<broadcast::Receiver<SessionCoordinationEvent>, Error> {
        if let Some(broadcaster) = self.broadcasters.get(context_id) {
            Ok(broadcaster.subscribe())
        } else {
            Err(Error::InternalError(
                format!("Context {} not found", context_id),
                ErrorContext::default().with_message("Context not found")
            ))
        }
    }
    
    /// Handle session termination
    pub async fn handle_session_termination(&self, session_id: SessionId) -> Result<(), Error> {
        // Propagate session termination event
        let event = SessionCoordinationEvent::StateChanged {
            session_id,
            old_state: SessionState::Connected, // Assume was connected
            new_state: SessionState::Terminated,
            timestamp: SystemTime::now(),
        };
        
        self.propagate_event(event, None, Some(100)).await?; // High priority
        
        // Remove session from all contexts
        if let Some((_, contexts)) = self.session_contexts.remove(&session_id) {
            for context in contexts {
                info!("🔄 Removed terminated session {} from context {}", session_id, context);
            }
        }
        
        info!("✅ Completed event propagation cleanup for session {}", session_id);
        Ok(())
    }
    
    /// Remove a propagation context
    pub async fn remove_context(&self, context_id: &str) -> Result<(), Error> {
        // Remove broadcaster
        self.broadcasters.remove(context_id);
        
        // Remove propagation rules and filters
        self.propagation_rules.remove(context_id);
        self.event_filters.remove(context_id);
        
        // Remove sessions from this context
        let mut sessions_to_update = Vec::new();
        for entry in self.session_contexts.iter() {
            if entry.value().contains(context_id) {
                sessions_to_update.push(entry.key().clone());
            }
        }
        
        for session_id in sessions_to_update {
            if let Some(mut contexts) = self.session_contexts.get_mut(&session_id) {
                contexts.remove(context_id);
                if contexts.is_empty() {
                    drop(contexts);
                    self.session_contexts.remove(&session_id);
                }
            }
        }
        
        info!("🗑️ Removed propagation context {}", context_id);
        Ok(())
    }
    
    /// Get propagation metrics
    pub async fn get_metrics(&self) -> PropagationMetrics {
        self.metrics.read().await.clone()
    }
    
    /// Get active context count
    pub async fn get_active_context_count(&self) -> usize {
        self.broadcasters.len()
    }
    
    /// Get sessions in a context
    pub async fn get_context_sessions(&self, context_id: &str) -> Vec<SessionId> {
        let mut sessions = Vec::new();
        
        for entry in self.session_contexts.iter() {
            if entry.value().contains(context_id) {
                sessions.push(entry.key().clone());
            }
        }
        
        sessions
    }
} 