//! Broadcast hub for WebSocket event distribution.

use serde::Serialize;
use tokio::sync::broadcast;
use tracing::debug;

/// Central hub that distributes events to all connected WebSocket clients.
#[derive(Debug, Clone)]
pub struct WsHub {
    tx: broadcast::Sender<String>,
}

/// An event pushed to frontend clients via WebSocket.
#[derive(Debug, Clone, Serialize)]
pub struct WsEvent {
    pub event_type: String,
    pub timestamp: String,
    pub data: serde_json::Value,
}

impl WsHub {
    /// Create a new hub with the given channel capacity.
    pub fn new(capacity: usize) -> Self {
        let (tx, _) = broadcast::channel(capacity);
        Self { tx }
    }

    /// Subscribe to the event stream.
    pub fn subscribe(&self) -> broadcast::Receiver<String> {
        self.tx.subscribe()
    }

    /// Broadcast a typed event to all connected clients.
    pub fn broadcast(&self, event: WsEvent) {
        if let Ok(json) = serde_json::to_string(&event) {
            let receivers = self.tx.send(json).unwrap_or(0);
            debug!(receivers, event_type = %event.event_type, "Broadcast event");
        }
    }

    /// Broadcast a raw JSON string.
    pub fn broadcast_raw(&self, json: String) {
        let _ = self.tx.send(json);
    }

    /// Number of active subscribers.
    pub fn subscriber_count(&self) -> usize {
        self.tx.receiver_count()
    }
}
