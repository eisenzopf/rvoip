//! Session Event System
//!
//! Integrates with infra-common zero-copy event system for high-performance session event handling.

use std::sync::Arc;
use std::any::Any;
use tokio::sync::RwLock;
use serde::{Serialize, Deserialize};
use infra_common::events::{
    types::{Event, EventPriority, EventResult},
    system::EventSystem,
    builder::{EventSystemBuilder, ImplementationType},
    api::{EventSystem as EventSystemTrait, EventSubscriber},
};
use crate::api::types::{SessionId, CallSession, CallState};
use crate::errors::Result;

/// Session events that can be published through the event system
#[derive(Debug, Clone, Serialize, Deserialize)]
pub enum SessionEvent {
    /// Session was created
    SessionCreated { 
        session_id: SessionId, 
        from: String,
        to: String,
        call_state: CallState,
    },
    
    /// Session state changed
    StateChanged { 
        session_id: SessionId, 
        old_state: CallState, 
        new_state: CallState,
    },
    
    /// Session was terminated
    SessionTerminated { 
        session_id: SessionId, 
        reason: String,
    },
    
    /// Media event
    MediaEvent { 
        session_id: SessionId, 
        event: String,
    },
    
    /// DTMF digits received
    DtmfReceived {
        session_id: SessionId,
        digits: String,
    },
    
    /// Session was held
    SessionHeld {
        session_id: SessionId,
    },
    
    /// Session was resumed from hold
    SessionResumed {
        session_id: SessionId,
    },
    
    /// Media update requested (e.g., re-INVITE with new SDP)
    MediaUpdate {
        session_id: SessionId,
        offered_sdp: Option<String>,
    },
    
    /// SDP event (offer, answer, or update)
    SdpEvent {
        session_id: SessionId,
        event_type: String, // "local_sdp_offer", "remote_sdp_answer", "sdp_update", etc.
        sdp: String,
    },
    
    /// Error event
    Error { 
        session_id: Option<SessionId>, 
        error: String,
    },
}

impl Event for SessionEvent {
    fn event_type() -> &'static str {
        "session_event"
    }
    
    fn priority() -> EventPriority {
        // Default priority for all session events, individual events can override if needed
        EventPriority::Normal
    }
    
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// Event processor for session events using infra-common zero-copy event system
pub struct SessionEventProcessor {
    event_system: EventSystem,
    publisher: Arc<RwLock<Option<Box<dyn infra_common::events::api::EventPublisher<SessionEvent> + Send + Sync>>>>,
}

impl std::fmt::Debug for SessionEventProcessor {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionEventProcessor")
            .field("has_publisher", &self.publisher.try_read().map(|p| p.is_some()).unwrap_or(false))
            .finish()
    }
}

impl SessionEventProcessor {
    /// Create a new session event processor
    pub fn new() -> Self {
        let event_system = EventSystemBuilder::new()
            .implementation(ImplementationType::ZeroCopy)
            .channel_capacity(10_000)
            .max_concurrent_dispatches(1_000)
            .enable_priority(true)
            .shard_count(8)
            .enable_metrics(false)
            .build();

        Self {
            event_system,
            publisher: Arc::new(RwLock::new(None)),
        }
    }

    /// Start the event processor
    pub async fn start(&self) -> Result<()> {
        self.event_system.start()
            .await
            .map_err(|e| crate::errors::SessionError::internal(&format!("Failed to start event system: {}", e)))?;
        
        // Create publisher
        let publisher = self.event_system.create_publisher::<SessionEvent>();
        *self.publisher.write().await = Some(publisher);

        tracing::info!("Session event processor started");
        Ok(())
    }

    /// Stop the event processor
    pub async fn stop(&self) -> Result<()> {
        *self.publisher.write().await = None;
        
        self.event_system.shutdown()
            .await
            .map_err(|e| crate::errors::SessionError::internal(&format!("Failed to stop event system: {}", e)))?;

        tracing::info!("Session event processor stopped");
        Ok(())
    }

    /// Publish a session event
    pub async fn publish_event(&self, event: SessionEvent) -> Result<()> {
        let publisher = self.publisher.read().await;
        if let Some(publisher) = publisher.as_ref() {
            publisher.publish(event)
                .await
                .map_err(|e| crate::errors::SessionError::internal(&format!("Failed to publish event: {}", e)))?;
        } else {
            tracing::warn!("Event processor not running, dropping event");
        }
        Ok(())
    }

    /// Subscribe to session events (for testing and monitoring)
    pub async fn subscribe(&self) -> Result<Box<dyn EventSubscriber<SessionEvent> + Send>> {
        let subscriber = self.event_system.subscribe::<SessionEvent>()
            .await
            .map_err(|e| crate::errors::SessionError::internal(&format!("Failed to subscribe to events: {}", e)))?;
        
        Ok(subscriber)
    }

    /// Subscribe to session events with a filter
    pub async fn subscribe_filtered<F>(&self, filter: F) -> Result<Box<dyn EventSubscriber<SessionEvent> + Send>>
    where
        F: Fn(&SessionEvent) -> bool + Send + Sync + 'static,
    {
        let subscriber = self.event_system.subscribe_filtered::<SessionEvent, F>(filter)
            .await
            .map_err(|e| crate::errors::SessionError::internal(&format!("Failed to subscribe to filtered events: {}", e)))?;
        
        Ok(subscriber)
    }

    /// Check if the event processor is running
    pub async fn is_running(&self) -> bool {
        self.publisher.read().await.is_some()
    }
}

impl Default for SessionEventProcessor {
    fn default() -> Self {
        Self::new()
    }
} 