//! Event handling utilities for RVOIP Simple

// Re-export main event types from lib.rs
pub use crate::{ClientEvent, CallEvent, IncomingCall, CallQuality};

use tokio::sync::broadcast;
use tracing::warn;

/// Event handler trait for processing VoIP events
pub trait EventHandler: Send + Sync {
    /// Handle a client event
    fn handle_client_event(&self, event: ClientEvent);
    
    /// Handle a call event
    fn handle_call_event(&self, event: CallEvent);
}

/// Async event handler trait
#[async_trait::async_trait]
pub trait AsyncEventHandler: Send + Sync {
    /// Handle a client event asynchronously
    async fn handle_client_event(&self, event: ClientEvent);
    
    /// Handle a call event asynchronously
    async fn handle_call_event(&self, event: CallEvent);
}

/// Simple event listener that processes events with callbacks
pub struct EventListener<F1, F2> 
where
    F1: Fn(ClientEvent) + Send + Sync + 'static,
    F2: Fn(CallEvent) + Send + Sync + 'static,
{
    client_handler: F1,
    call_handler: F2,
}

impl<F1, F2> EventListener<F1, F2>
where
    F1: Fn(ClientEvent) + Send + Sync + 'static,
    F2: Fn(CallEvent) + Send + Sync + 'static,
{
    /// Create a new event listener with callback functions
    pub fn new(client_handler: F1, call_handler: F2) -> Self {
        Self {
            client_handler,
            call_handler,
        }
    }
    
    /// Process client events from a receiver
    pub async fn listen_client_events(&self, mut rx: broadcast::Receiver<ClientEvent>) {
        while let Ok(event) = rx.recv().await {
            (self.client_handler)(event);
        }
    }
    
    /// Process call events from a receiver
    pub async fn listen_call_events(&self, mut rx: broadcast::Receiver<CallEvent>) {
        while let Ok(event) = rx.recv().await {
            (self.call_handler)(event);
        }
    }
}

/// Convenience functions for common event handling patterns
impl ClientEvent {
    /// Check if this is an error event
    pub fn is_error(&self) -> bool {
        matches!(self, 
            ClientEvent::RegistrationFailed(_) |
            ClientEvent::NetworkError(_)
        )
    }
    
    /// Check if this is a success event
    pub fn is_success(&self) -> bool {
        matches!(self, ClientEvent::RegistrationSuccess)
    }
    
    /// Get error message if this is an error event
    pub fn error_message(&self) -> Option<&str> {
        match self {
            ClientEvent::RegistrationFailed(msg) => Some(msg),
            ClientEvent::NetworkError(msg) => Some(msg),
            _ => None,
        }
    }
}

impl CallEvent {
    /// Check if this is a call state change event
    pub fn is_state_change(&self) -> bool {
        matches!(self, CallEvent::StateChanged(_, _))
    }
    
    /// Check if this is a media-related event
    pub fn is_media_event(&self) -> bool {
        matches!(self, 
            CallEvent::MediaConnected(_) |
            CallEvent::MediaDisconnected(_) |
            CallEvent::QualityChanged(_, _)
        )
    }
    
    /// Get the call ID associated with this event
    pub fn call_id(&self) -> Option<&str> {
        match self {
            CallEvent::StateChanged(id, _) => Some(id),
            CallEvent::MediaConnected(id) => Some(id),
            CallEvent::MediaDisconnected(id) => Some(id),
            CallEvent::QualityChanged(id, _) => Some(id),
            CallEvent::DtmfReceived(id, _) => Some(id),
            CallEvent::Answered => None,
            CallEvent::Ended => None,
        }
    }
}

/// Event filtering utilities
pub struct EventFilter;

impl EventFilter {
    /// Filter events by call ID
    pub fn by_call_id(call_id: &str) -> impl Fn(&CallEvent) -> bool + '_ {
        move |event: &CallEvent| {
            event.call_id().map(|id| id == call_id).unwrap_or(false)
        }
    }
    
    /// Filter for error events only
    pub fn errors_only() -> impl Fn(&ClientEvent) -> bool {
        |event: &ClientEvent| event.is_error()
    }
    
    /// Filter for media quality events
    pub fn quality_events() -> impl Fn(&CallEvent) -> bool {
        |event: &CallEvent| matches!(event, CallEvent::QualityChanged(_, _))
    }
    
    /// Filter for DTMF events
    pub fn dtmf_events() -> impl Fn(&CallEvent) -> bool {
        |event: &CallEvent| matches!(event, CallEvent::DtmfReceived(_, _))
    }
}

/// Event statistics collector
#[derive(Debug, Default)]
pub struct EventStats {
    pub client_events: u64,
    pub call_events: u64,
    pub error_events: u64,
    pub quality_events: u64,
}

impl EventStats {
    /// Record a client event
    pub fn record_client_event(&mut self, event: &ClientEvent) {
        self.client_events += 1;
        if event.is_error() {
            self.error_events += 1;
        }
    }
    
    /// Record a call event
    pub fn record_call_event(&mut self, event: &CallEvent) {
        self.call_events += 1;
        if matches!(event, CallEvent::QualityChanged(_, _)) {
            self.quality_events += 1;
        }
    }
    
    /// Get total events processed
    pub fn total_events(&self) -> u64 {
        self.client_events + self.call_events
    }
    
    /// Get error rate as percentage
    pub fn error_rate(&self) -> f64 {
        if self.client_events == 0 {
            0.0
        } else {
            (self.error_events as f64 / self.client_events as f64) * 100.0
        }
    }
} 