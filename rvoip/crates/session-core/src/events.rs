use std::sync::Arc;
use async_trait::async_trait;
use tokio::sync::broadcast;
use uuid::Uuid;

use crate::session::{SessionId, SessionState};
use crate::dialog::DialogId;

/// Event types that can be emitted during session lifecycle
#[derive(Debug, Clone)]
pub enum SessionEvent {
    /// A new session was created
    Created {
        session_id: SessionId,
    },
    
    /// Session state changed
    StateChanged {
        session_id: SessionId,
        old_state: SessionState,
        new_state: SessionState,
    },
    
    /// Dialog was created or updated
    DialogUpdated {
        session_id: SessionId,
        dialog_id: DialogId,
    },
    
    /// Media stream started
    MediaStarted {
        session_id: SessionId,
    },
    
    /// Media stream stopped
    MediaStopped {
        session_id: SessionId,
    },
    
    /// DTMF digit received
    DtmfReceived {
        session_id: SessionId,
        digit: char,
    },
    
    /// Session terminated
    Terminated {
        session_id: SessionId,
        reason: String,
    },
    
    /// Custom event type for application-specific events
    Custom {
        session_id: SessionId,
        event_type: String,
        data: serde_json::Value,
    },
}

/// Trait for handling session events
#[async_trait]
pub trait EventHandler: Send + Sync {
    /// Handle a session event
    async fn handle_event(&self, event: SessionEvent);
}

/// Event bus for broadcasting session events
#[derive(Clone)]
pub struct EventBus {
    sender: broadcast::Sender<SessionEvent>,
}

impl EventBus {
    /// Create a new event bus
    pub fn new(capacity: usize) -> Self {
        let (sender, _) = broadcast::channel(capacity);
        Self { sender }
    }
    
    /// Subscribe to events
    pub fn subscribe(&self) -> broadcast::Receiver<SessionEvent> {
        self.sender.subscribe()
    }
    
    /// Publish an event
    pub fn publish(&self, event: SessionEvent) {
        let _ = self.sender.send(event);
    }
    
    /// Register an event handler
    pub async fn register_handler(&self, handler: Arc<dyn EventHandler>) -> broadcast::Receiver<SessionEvent> {
        let mut rx = self.subscribe();
        let handler_clone = handler.clone();
        
        tokio::spawn(async move {
            while let Ok(event) = rx.recv().await {
                handler_clone.handle_event(event.clone()).await;
            }
        });
        
        self.subscribe()
    }
    
    /// Create a default event bus
    pub fn default() -> Self {
        Self::new(100)
    }
} 