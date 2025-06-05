//! Event Bus
//!
//! Simple event bus for session events.

use tokio::sync::mpsc;
use crate::errors::Result;
use super::types::SessionEvent;

/// Event bus for session events
#[derive(Debug)]
pub struct EventBus {
    sender: mpsc::UnboundedSender<SessionEvent>,
    _receiver: mpsc::UnboundedReceiver<SessionEvent>,
}

impl EventBus {
    pub fn new(capacity: usize) -> Self {
        let (sender, receiver) = mpsc::unbounded_channel();
        Self {
            sender,
            _receiver: receiver,
        }
    }

    pub async fn publish(&self, event: SessionEvent) -> Result<()> {
        self.sender.send(event).map_err(|e| crate::errors::SessionError::Other(format!("Failed to publish event: {}", e)))?;
        Ok(())
    }

    pub fn get_sender(&self) -> mpsc::UnboundedSender<SessionEvent> {
        self.sender.clone()
    }
} 