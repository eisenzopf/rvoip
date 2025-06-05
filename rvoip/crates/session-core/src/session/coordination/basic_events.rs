//! Basic Event Primitives
//! 
//! This module provides low-level event communication primitives for session coordination.
//! Complex event orchestration and business logic is handled by higher layers (call-engine).
//! 
//! ## Scope
//! 
//! **✅ Included (Basic Primitives)**:
//! - Simple session event types
//! - Basic pub/sub event bus
//! - Simple event publishing and subscription
//! - Basic event filtering (session-to-session only)
//! 
//! **❌ Not Included (Business Logic - belongs in call-engine)**:
//! - Complex event routing and propagation rules
//! - Event orchestration and transformation logic
//! - Business event coordination and filtering
//! - Advanced metrics and loop prevention

use std::collections::HashMap;
use std::sync::Arc;
use std::time::SystemTime;
use tokio::sync::broadcast;
use serde::{Serialize, Deserialize};

use crate::session::{SessionId, SessionState};

/// Basic session events (simple classification)
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum BasicSessionEvent {
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
    
    /// Session terminated
    SessionTerminated {
        session_id: SessionId,
        reason: String,
        timestamp: SystemTime,
    },
    
    /// Custom session event
    Custom {
        event_type: String,
        session_id: SessionId,
        data: HashMap<String, String>,
        timestamp: SystemTime,
    },
}

impl BasicSessionEvent {
    /// Get the session ID for this event
    pub fn session_id(&self) -> SessionId {
        match self {
            BasicSessionEvent::StateChanged { session_id, .. } => *session_id,
            BasicSessionEvent::MediaStateChanged { session_id, .. } => *session_id,
            BasicSessionEvent::SessionTerminated { session_id, .. } => *session_id,
            BasicSessionEvent::Custom { session_id, .. } => *session_id,
        }
    }
    
    /// Get the event type as string
    pub fn event_type(&self) -> &str {
        match self {
            BasicSessionEvent::StateChanged { .. } => "StateChanged",
            BasicSessionEvent::MediaStateChanged { .. } => "MediaStateChanged",
            BasicSessionEvent::SessionTerminated { .. } => "SessionTerminated",
            BasicSessionEvent::Custom { event_type, .. } => event_type,
        }
    }
    
    /// Create a state change event
    pub fn state_changed(
        session_id: SessionId,
        old_state: SessionState,
        new_state: SessionState,
    ) -> Self {
        BasicSessionEvent::StateChanged {
            session_id,
            old_state,
            new_state,
            timestamp: SystemTime::now(),
        }
    }
    
    /// Create a media state change event
    pub fn media_state_changed(session_id: SessionId, media_state: String) -> Self {
        BasicSessionEvent::MediaStateChanged {
            session_id,
            media_state,
            timestamp: SystemTime::now(),
        }
    }
    
    /// Create a session termination event
    pub fn session_terminated(session_id: SessionId, reason: String) -> Self {
        BasicSessionEvent::SessionTerminated {
            session_id,
            reason,
            timestamp: SystemTime::now(),
        }
    }
}

/// Basic event bus configuration
#[derive(Debug, Clone)]
pub struct BasicEventBusConfig {
    /// Maximum events to buffer
    pub max_buffer_size: usize,
    
    /// Whether to log events
    pub log_events: bool,
}

impl Default for BasicEventBusConfig {
    fn default() -> Self {
        Self {
            max_buffer_size: 1000,
            log_events: false,
        }
    }
}

/// Basic event bus for simple session-to-session communication
pub struct BasicEventBus {
    /// Event broadcaster
    broadcaster: broadcast::Sender<BasicSessionEvent>,
    
    /// Configuration
    config: BasicEventBusConfig,
}

impl BasicEventBus {
    /// Create a new basic event bus
    pub fn new(config: BasicEventBusConfig) -> Self {
        let (broadcaster, _) = broadcast::channel(config.max_buffer_size);
        
        Self {
            broadcaster,
            config,
        }
    }
    
    /// Create with default configuration
    pub fn with_defaults() -> Self {
        Self::new(BasicEventBusConfig::default())
    }
    
    /// Publish an event to all subscribers
    pub fn publish(&self, event: BasicSessionEvent) -> Result<usize, broadcast::error::SendError<BasicSessionEvent>> {
        if self.config.log_events {
            tracing::debug!("📡 Publishing event: {:?}", event);
        }
        
        self.broadcaster.send(event)
    }
    
    /// Subscribe to events
    pub fn subscribe(&self) -> broadcast::Receiver<BasicSessionEvent> {
        self.broadcaster.subscribe()
    }
    
    /// Get the number of active subscribers
    pub fn subscriber_count(&self) -> usize {
        self.broadcaster.receiver_count()
    }
    
    /// Check if there are any subscribers
    pub fn has_subscribers(&self) -> bool {
        self.broadcaster.receiver_count() > 0
    }
}

/// Helper for basic event filtering (session-based only)
#[derive(Debug, Clone)]
pub struct BasicEventFilter {
    /// Include only these session IDs (empty = all)
    pub include_sessions: Option<Vec<SessionId>>,
    
    /// Exclude these session IDs
    pub exclude_sessions: Vec<SessionId>,
    
    /// Include only these event types (empty = all)
    pub include_event_types: Option<Vec<String>>,
    
    /// Exclude these event types
    pub exclude_event_types: Vec<String>,
}

impl Default for BasicEventFilter {
    fn default() -> Self {
        Self {
            include_sessions: None,
            exclude_sessions: Vec::new(),
            include_event_types: None,
            exclude_event_types: Vec::new(),
        }
    }
}

impl BasicEventFilter {
    /// Check if an event passes this filter
    pub fn matches(&self, event: &BasicSessionEvent) -> bool {
        let session_id = event.session_id();
        let event_type = event.event_type();
        
        // Check session inclusion
        if let Some(ref include_sessions) = self.include_sessions {
            if !include_sessions.contains(&session_id) {
                return false;
            }
        }
        
        // Check session exclusion
        if self.exclude_sessions.contains(&session_id) {
            return false;
        }
        
        // Check event type inclusion
        if let Some(ref include_types) = self.include_event_types {
            if !include_types.contains(&event_type.to_string()) {
                return false;
            }
        }
        
        // Check event type exclusion
        if self.exclude_event_types.contains(&event_type.to_string()) {
            return false;
        }
        
        true
    }
    
    /// Create a filter for specific sessions
    pub fn for_sessions(session_ids: Vec<SessionId>) -> Self {
        Self {
            include_sessions: Some(session_ids),
            ..Default::default()
        }
    }
    
    /// Create a filter for specific event types
    pub fn for_event_types(event_types: Vec<String>) -> Self {
        Self {
            include_event_types: Some(event_types),
            ..Default::default()
        }
    }
}

/// Filtered event subscriber that only receives matching events
pub struct FilteredEventSubscriber {
    /// Base event receiver
    receiver: broadcast::Receiver<BasicSessionEvent>,
    
    /// Event filter
    filter: BasicEventFilter,
}

impl FilteredEventSubscriber {
    /// Create a new filtered subscriber
    pub fn new(
        event_bus: &BasicEventBus,
        filter: BasicEventFilter,
    ) -> Self {
        Self {
            receiver: event_bus.subscribe(),
            filter,
        }
    }
    
    /// Receive the next matching event
    pub async fn recv(&mut self) -> Result<BasicSessionEvent, broadcast::error::RecvError> {
        loop {
            let event = self.receiver.recv().await?;
            if self.filter.matches(&event) {
                return Ok(event);
            }
            // Continue loop to get next event if this one doesn't match
        }
    }
    
    /// Try to receive a matching event without blocking
    pub fn try_recv(&mut self) -> Result<BasicSessionEvent, broadcast::error::TryRecvError> {
        loop {
            let event = self.receiver.try_recv()?;
            if self.filter.matches(&event) {
                return Ok(event);
            }
            // Continue loop to get next event if this one doesn't match
        }
    }
} 