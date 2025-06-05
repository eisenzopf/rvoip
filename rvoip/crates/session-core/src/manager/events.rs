//! Event Processor
//!
//! Handles processing of session-related events.

use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use crate::api::types::{SessionId, CallSession};
use crate::errors::Result;

/// Event processor for session events
#[derive(Debug)]
pub struct EventProcessor {
    event_sender: Arc<RwLock<Option<mpsc::UnboundedSender<SessionEvent>>>>,
    is_running: Arc<RwLock<bool>>,
}

/// Session events that can be processed
#[derive(Debug, Clone)]
pub enum SessionEvent {
    /// Session was created
    SessionCreated { session_id: SessionId, session: CallSession },
    
    /// Session state changed
    StateChanged { session_id: SessionId, old_state: String, new_state: String },
    
    /// Session was terminated
    SessionTerminated { session_id: SessionId, reason: String },
    
    /// Media event
    MediaEvent { session_id: SessionId, event: String },
    
    /// Error event
    Error { session_id: Option<SessionId>, error: String },
}

impl EventProcessor {
    /// Create a new event processor
    pub fn new() -> Self {
        Self {
            event_sender: Arc::new(RwLock::new(None)),
            is_running: Arc::new(RwLock::new(false)),
        }
    }

    /// Start the event processor
    pub async fn start(&self) -> Result<()> {
        let mut is_running = self.is_running.write().await;
        if *is_running {
            return Ok(()); // Already running
        }

        let (sender, mut receiver) = mpsc::unbounded_channel();
        *self.event_sender.write().await = Some(sender);
        *is_running = true;

        // Spawn event processing task
        tokio::spawn(async move {
            while let Some(event) = receiver.recv().await {
                if let Err(e) = Self::process_event(event).await {
                    tracing::error!("Error processing event: {}", e);
                }
            }
        });

        tracing::info!("Event processor started");
        Ok(())
    }

    /// Stop the event processor
    pub async fn stop(&self) -> Result<()> {
        let mut is_running = self.is_running.write().await;
        if !*is_running {
            return Ok(()); // Already stopped
        }

        *self.event_sender.write().await = None;
        *is_running = false;

        tracing::info!("Event processor stopped");
        Ok(())
    }

    /// Send an event for processing
    pub async fn send_event(&self, event: SessionEvent) -> Result<()> {
        let sender = self.event_sender.read().await;
        if let Some(sender) = sender.as_ref() {
            sender.send(event).map_err(|e| crate::errors::SessionError::Other(format!("Failed to send event: {}", e)))?;
        } else {
            tracing::warn!("Event processor not running, dropping event");
        }
        Ok(())
    }

    /// Process a single event
    async fn process_event(event: SessionEvent) -> Result<()> {
        match event {
            SessionEvent::SessionCreated { session_id, .. } => {
                tracing::debug!("Processing session created: {}", session_id);
                // TODO: Notify handlers, update metrics, etc.
            }
            
            SessionEvent::StateChanged { session_id, old_state, new_state } => {
                tracing::debug!("Session {} state changed: {} -> {}", session_id, old_state, new_state);
                // TODO: Update session state, notify handlers
            }
            
            SessionEvent::SessionTerminated { session_id, reason } => {
                tracing::debug!("Session {} terminated: {}", session_id, reason);
                // TODO: Cleanup resources, notify handlers
            }
            
            SessionEvent::MediaEvent { session_id, event } => {
                tracing::debug!("Media event for session {}: {}", session_id, event);
                // TODO: Handle media events
            }
            
            SessionEvent::Error { session_id, error } => {
                if let Some(session_id) = session_id {
                    tracing::error!("Error for session {}: {}", session_id, error);
                } else {
                    tracing::error!("General error: {}", error);
                }
                // TODO: Handle errors, possibly terminate sessions
            }
        }
        Ok(())
    }

    /// Check if the event processor is running
    pub async fn is_running(&self) -> bool {
        *self.is_running.read().await
    }
}

impl Default for EventProcessor {
    fn default() -> Self {
        Self::new()
    }
} 