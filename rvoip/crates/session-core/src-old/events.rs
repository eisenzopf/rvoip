//! Session event system using zero-copy event infrastructure
//!
//! This module provides session-specific events using the high-performance
//! zero-copy event system from infra-common.

use std::sync::Arc;
use std::any::Any;
use std::fmt;
use async_trait::async_trait;
use serde::{Serialize, Deserialize};
use uuid::Uuid;

// Import zero-copy event system
use infra_common::events::system::{EventSystem, EventPublisher, EventSubscriber};
use infra_common::events::builder::{EventSystemBuilder, ImplementationType};
use infra_common::events::types::{Event, EventPriority, EventResult, EventFilter};
use infra_common::events::bus::EventBusConfig;
use infra_common::events::api::EventSystem as EventSystemTrait;

use rvoip_sip_core::{Response, StatusCode};

use crate::session::{SessionId, SessionState};
use crate::dialog::{DialogId, DialogState};
use crate::sdp::NegotiationState;

/// Session event types for the SIP session management
#[derive(Debug, Clone, Serialize, Deserialize)]
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
    
    /// Provisional response received
    ProvisionalResponse {
        session_id: SessionId,
        status_code: u16,
        reason_phrase: String,
    },
    
    /// Success response received
    SuccessResponse {
        session_id: SessionId,
        status_code: u16,
        reason_phrase: String,
    },
    
    /// Failure response received
    FailureResponse {
        session_id: SessionId,
        status_code: u16,
        reason_phrase: String,
    },
    
    /// New transaction created
    TransactionCreated {
        session_id: SessionId,
        transaction_id: String,
        is_client: bool,
        method: String,
    },
    
    /// Transaction completed
    TransactionCompleted {
        session_id: SessionId,
        transaction_id: String,
        success: bool,
    },
    
    /// Dialog created
    DialogCreated {
        session_id: SessionId,
        dialog_id: DialogId,
    },
    
    /// Dialog state changed
    DialogStateChanged {
        session_id: SessionId,
        dialog_id: DialogId,
        previous: DialogState,
        current: DialogState,
    },
    
    /// SDP offer sent in a request
    SdpOfferSent {
        session_id: SessionId,
        dialog_id: DialogId,
    },
    
    /// SDP offer received in a request
    SdpOfferReceived {
        session_id: SessionId,
        dialog_id: DialogId,
    },
    
    /// SDP answer sent in a response
    SdpAnswerSent {
        session_id: SessionId,
        dialog_id: DialogId,
    },
    
    /// SDP answer received in a response
    SdpAnswerReceived {
        session_id: SessionId,
        dialog_id: DialogId,
    },
    
    /// SDP negotiation completed
    SdpNegotiationComplete {
        session_id: SessionId,
        dialog_id: DialogId,
    },
    
    /// Dialog recovery started
    DialogRecoveryStarted {
        session_id: SessionId,
        dialog_id: DialogId,
        reason: String,
    },
    
    /// Dialog recovery completed (successfully or not)
    DialogRecoveryCompleted {
        session_id: SessionId,
        dialog_id: DialogId,
        success: bool,
    },
    
    // ==== Transfer Events (REFER Method Support) ====
    
    /// Transfer request initiated (REFER sent/received)
    TransferInitiated {
        session_id: SessionId,
        transfer_id: String,
        transfer_type: String,
        target_uri: String,
    },
    
    /// Transfer accepted (202 Accepted sent/received)
    TransferAccepted {
        session_id: SessionId,
        transfer_id: String,
    },
    
    /// Transfer progress notification (NOTIFY received/sent)
    TransferProgress {
        session_id: SessionId,
        transfer_id: String,
        status: String,
    },
    
    /// Transfer completed successfully
    TransferCompleted {
        session_id: SessionId,
        transfer_id: String,
        final_status: String,
    },
    
    /// Transfer failed
    TransferFailed {
        session_id: SessionId,
        transfer_id: String,
        reason: String,
    },
    
    /// Consultation call created for attended transfer
    ConsultationCallCreated {
        original_session_id: SessionId,
        consultation_session_id: SessionId,
        transfer_id: String,
    },
    
    /// Consultation call completed
    ConsultationCallCompleted {
        original_session_id: SessionId,
        consultation_session_id: SessionId,
        transfer_id: String,
        success: bool,
    },
    
    /// Custom event type for application-specific events
    Custom {
        session_id: SessionId,
        event_type: String,
        data: serde_json::Value,
    },
}

// Implement the Event trait for zero-copy support
impl Event for SessionEvent {
    fn event_type() -> &'static str {
        "session_event"
    }
    
    fn priority() -> EventPriority {
        EventPriority::Normal
    }
    
    fn as_any(&self) -> &dyn Any {
        self
    }
}

/// High-performance event bus using zero-copy event system
pub struct EventBus {
    event_system: EventSystem,
    publisher: EventPublisher<SessionEvent>,
}

impl Clone for EventBus {
    fn clone(&self) -> Self {
        // Create a new publisher from the cloned event system
        let publisher_box = self.event_system.create_publisher::<SessionEvent>();
        let publisher = EventPublisher::new(publisher_box);
        
        Self {
            event_system: self.event_system.clone(),
            publisher,
        }
    }
}

impl EventBus {
    /// Create a new event bus with zero-copy event system
    pub async fn new(capacity: usize) -> Result<Self, Box<dyn std::error::Error>> {
        let event_system = EventSystemBuilder::new()
            .implementation(ImplementationType::ZeroCopy)
            .channel_capacity(capacity)
            .max_concurrent_dispatches(capacity / 2)
            .enable_priority(true)
            .default_timeout(Some(std::time::Duration::from_secs(5)))
            .batch_size(100)
            .shard_count(8)
            .build();
        
        // Start the event system
        event_system.start().await?;
        
        // Create publisher
        let publisher_box = event_system.create_publisher::<SessionEvent>();
        let publisher = EventPublisher::new(publisher_box);
        
        Ok(Self {
            event_system,
            publisher,
        })
    }
    
    /// Create a new event bus with default configuration
    pub async fn default() -> Result<Self, Box<dyn std::error::Error>> {
        Self::new(1000).await
    }
    
    /// Create a simple event bus for testing (synchronous)
    pub fn new_simple(capacity: usize) -> Self {
        // Create a runtime for the async initialization
        let rt = tokio::runtime::Handle::try_current()
            .or_else(|_| {
                // If no runtime is available, create a temporary one
                tokio::runtime::Runtime::new().map(|rt| rt.handle().clone())
            })
            .expect("No tokio runtime available");
        
        rt.block_on(async {
            Self::new(capacity).await.expect("Failed to create event bus")
        })
    }
    
    /// Subscribe to session events
    pub async fn subscribe(&self) -> Result<EventSubscriber<SessionEvent>, Box<dyn std::error::Error>> {
        let subscriber_box = self.event_system.subscribe::<SessionEvent>().await?;
        Ok(EventSubscriber::new(subscriber_box))
    }
    
    /// Subscribe with a filter
    pub async fn subscribe_filtered<F>(&self, filter: F) -> Result<EventSubscriber<SessionEvent>, Box<dyn std::error::Error>>
    where
        F: Fn(&SessionEvent) -> bool + Send + Sync + 'static,
    {
        let subscriber_box = self.event_system.subscribe_filtered(filter).await?;
        Ok(EventSubscriber::new(subscriber_box))
    }
    
    /// Publish an event
    pub async fn publish(&self, event: SessionEvent) -> Result<(), Box<dyn std::error::Error>> {
        self.publisher.publish(event).await.map_err(|e| e.into())
    }
    
    /// Publish a batch of events for better performance
    pub async fn publish_batch(&self, events: Vec<SessionEvent>) -> Result<(), Box<dyn std::error::Error>> {
        self.publisher.publish_batch(events).await.map_err(|e| e.into())
    }
    
    /// Register an event handler with the zero-copy system
    pub async fn register_handler(&self, handler: Arc<dyn EventHandler>) -> Result<(), Box<dyn std::error::Error>> {
        let mut subscriber = self.subscribe().await?;
        
        tokio::spawn(async move {
            loop {
                match subscriber.receive().await {
                    Ok(event) => {
                        handler.handle_event((*event).clone()).await;
                    },
                    Err(_) => break, // Channel closed or error
                }
            }
        });
        
        Ok(())
    }
    
    /// Shutdown the event system
    pub async fn shutdown(&self) -> Result<(), Box<dyn std::error::Error>> {
        self.event_system.shutdown().await.map_err(|e| e.into())
    }
    
    /// Get event system metrics
    pub fn metrics(&self) -> serde_json::Value {
        // The zero-copy system can provide detailed metrics
        serde_json::json!({
            "system_type": "zero_copy",
            "implementation": "sharded"
        })
    }
}

/// Trait for handling session events
#[async_trait]
pub trait EventHandler: Send + Sync {
    /// Handle a session event
    async fn handle_event(&self, event: SessionEvent);
}

/// Session event priority for different types of events
impl SessionEvent {
    pub fn priority(&self) -> EventPriority {
        match self {
            // High priority events
            SessionEvent::Terminated { .. } |
            SessionEvent::TransferFailed { .. } |
            SessionEvent::FailureResponse { .. } => EventPriority::High,
            
            // Normal priority events
            SessionEvent::Created { .. } |
            SessionEvent::StateChanged { .. } |
            SessionEvent::TransferInitiated { .. } |
            SessionEvent::TransferCompleted { .. } |
            SessionEvent::ConsultationCallCreated { .. } |
            SessionEvent::TransferAccepted { .. } |
            SessionEvent::SuccessResponse { .. } |
            SessionEvent::TransactionCreated { .. } |
            SessionEvent::TransactionCompleted { .. } |
            SessionEvent::DialogCreated { .. } |
            SessionEvent::DialogStateChanged { .. } |
            SessionEvent::SdpOfferSent { .. } |
            SessionEvent::SdpOfferReceived { .. } |
            SessionEvent::SdpAnswerSent { .. } |
            SessionEvent::SdpAnswerReceived { .. } |
            SessionEvent::SdpNegotiationComplete { .. } |
            SessionEvent::DialogRecoveryStarted { .. } |
            SessionEvent::DialogRecoveryCompleted { .. } |
            SessionEvent::ConsultationCallCompleted { .. } => EventPriority::Normal,
            
            // Low priority events  
            SessionEvent::TransferProgress { .. } |
            SessionEvent::ProvisionalResponse { .. } |
            SessionEvent::DialogUpdated { .. } |
            SessionEvent::DtmfReceived { .. } |
            SessionEvent::Custom { .. } |
            SessionEvent::MediaStarted { .. } |
            SessionEvent::MediaStopped { .. } => EventPriority::Low,
        }
    }
    
    /// Get the session ID from any event
    pub fn session_id(&self) -> SessionId {
        match self {
            SessionEvent::Created { session_id } |
            SessionEvent::StateChanged { session_id, .. } |
            SessionEvent::DialogUpdated { session_id, .. } |
            SessionEvent::MediaStarted { session_id } |
            SessionEvent::MediaStopped { session_id } |
            SessionEvent::DtmfReceived { session_id, .. } |
            SessionEvent::Terminated { session_id, .. } |
            SessionEvent::ProvisionalResponse { session_id, .. } |
            SessionEvent::SuccessResponse { session_id, .. } |
            SessionEvent::FailureResponse { session_id, .. } |
            SessionEvent::TransactionCreated { session_id, .. } |
            SessionEvent::TransactionCompleted { session_id, .. } |
            SessionEvent::DialogCreated { session_id, .. } |
            SessionEvent::DialogStateChanged { session_id, .. } |
            SessionEvent::SdpOfferSent { session_id, .. } |
            SessionEvent::SdpOfferReceived { session_id, .. } |
            SessionEvent::SdpAnswerSent { session_id, .. } |
            SessionEvent::SdpAnswerReceived { session_id, .. } |
            SessionEvent::SdpNegotiationComplete { session_id, .. } |
            SessionEvent::DialogRecoveryStarted { session_id, .. } |
            SessionEvent::DialogRecoveryCompleted { session_id, .. } |
            SessionEvent::TransferInitiated { session_id, .. } |
            SessionEvent::TransferAccepted { session_id, .. } |
            SessionEvent::TransferProgress { session_id, .. } |
            SessionEvent::TransferCompleted { session_id, .. } |
            SessionEvent::TransferFailed { session_id, .. } |
            SessionEvent::Custom { session_id, .. } => session_id.clone(),
            SessionEvent::ConsultationCallCreated { original_session_id, .. } |
            SessionEvent::ConsultationCallCompleted { original_session_id, .. } => original_session_id.clone(),
        }
    }
}

/// Event filters for specific event types
pub struct EventFilters;

impl EventFilters {
    /// Filter for transfer-related events only
    pub fn transfers_only() -> impl Fn(&SessionEvent) -> bool {
        |event| matches!(event, 
            SessionEvent::TransferInitiated { .. } |
            SessionEvent::TransferAccepted { .. } |
            SessionEvent::TransferProgress { .. } |
            SessionEvent::TransferCompleted { .. } |
            SessionEvent::TransferFailed { .. } |
            SessionEvent::ConsultationCallCreated { .. } |
            SessionEvent::ConsultationCallCompleted { .. }
        )
    }
    
    /// Filter for specific session ID
    pub fn session_id_filter(target_session_id: SessionId) -> impl Fn(&SessionEvent) -> bool {
        move |event| event.session_id() == target_session_id
    }
    
    /// Filter for state change events only
    pub fn state_changes_only() -> impl Fn(&SessionEvent) -> bool {
        |event| matches!(event,
            SessionEvent::StateChanged { .. } |
            SessionEvent::DialogStateChanged { .. }
        )
    }
    
    /// Filter for high priority events only
    pub fn high_priority_only() -> impl Fn(&SessionEvent) -> bool {
        |event| matches!(event.priority(), EventPriority::High)
    }
}

// Unit tests for the zero-copy event system
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::time::{sleep, Duration};

    struct TestEventHandler {
        event_count: AtomicUsize,
        transfer_count: AtomicUsize,
    }

    impl TestEventHandler {
        fn new() -> Self {
            Self {
                event_count: AtomicUsize::new(0),
                transfer_count: AtomicUsize::new(0),
            }
        }
        
        fn event_count(&self) -> usize {
            self.event_count.load(Ordering::SeqCst)
        }
        
        fn transfer_count(&self) -> usize {
            self.transfer_count.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl EventHandler for TestEventHandler {
        async fn handle_event(&self, event: SessionEvent) {
            self.event_count.fetch_add(1, Ordering::SeqCst);
            
            if EventFilters::transfers_only()(&event) {
                self.transfer_count.fetch_add(1, Ordering::SeqCst);
            }
        }
    }

    #[tokio::test]
    async fn test_zero_copy_event_bus() {
        let event_bus = EventBus::new(100).await.unwrap();
        let session_id = SessionId::new();
        
        // Create subscriber
        let mut subscriber = event_bus.subscribe().await.unwrap();
        
        // Publish event
        let event = SessionEvent::Created { session_id: session_id.clone() };
        event_bus.publish(event).await.unwrap();
        
        // Receive event
        let received = subscriber.receive_timeout(Duration::from_millis(100)).await.unwrap();
        assert_eq!(received.session_id(), session_id);
        
        // Shutdown
        event_bus.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn test_transfer_events_zero_copy() {
        let event_bus = EventBus::new(100).await.unwrap();
        let session_id = SessionId::new();
        
        // Register handler
        let handler = Arc::new(TestEventHandler::new());
        event_bus.register_handler(handler.clone()).await.unwrap();
        
        // Publish transfer events
        let events = vec![
            SessionEvent::TransferInitiated {
                session_id: session_id.clone(),
                transfer_id: "transfer1".to_string(),
                transfer_type: "Blind".to_string(),
                target_uri: "sip:bob@example.com".to_string(),
            },
            SessionEvent::TransferProgress {
                session_id: session_id.clone(),
                transfer_id: "transfer1".to_string(),
                status: "100 Trying".to_string(),
            },
            SessionEvent::TransferCompleted {
                session_id: session_id.clone(),
                transfer_id: "transfer1".to_string(),
                final_status: "200 OK".to_string(),
            },
        ];
        
        // Batch publish for better performance
        event_bus.publish_batch(events).await.unwrap();
        
        // Wait for events to be processed
        sleep(Duration::from_millis(50)).await;
        
        assert_eq!(handler.event_count(), 3);
        assert_eq!(handler.transfer_count(), 3);
        
        event_bus.shutdown().await.unwrap();
    }

    #[tokio::test]
    async fn test_event_priority() {
        let session_id = SessionId::new();
        
        let high_priority_event = SessionEvent::Terminated {
            session_id: session_id.clone(),
            reason: "Connection lost".to_string(),
        };
        
        let low_priority_event = SessionEvent::TransferProgress {
            session_id: session_id.clone(),
            transfer_id: "transfer1".to_string(),
            status: "180 Ringing".to_string(),
        };
        
        assert_eq!(high_priority_event.priority(), EventPriority::High);
        assert_eq!(low_priority_event.priority(), EventPriority::Low);
    }
}

// Add Display implementation for the NegotiationState enum
impl fmt::Display for NegotiationState {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            NegotiationState::Initial => write!(f, "Initial"),
            NegotiationState::OfferSent => write!(f, "OfferSent"),
            NegotiationState::OfferReceived => write!(f, "OfferReceived"),
            NegotiationState::Complete => write!(f, "Complete"),
        }
    }
}

/// Add SDP-related events to the session context
#[derive(Debug, Clone)]
pub enum SdpEvent {
    /// SDP offer was sent
    OfferSent {
        /// Session ID
        session_id: String,
        /// Dialog ID
        dialog_id: String,
    },

    /// SDP offer was received
    OfferReceived {
        /// Session ID
        session_id: String,
        /// Dialog ID
        dialog_id: String,
    },

    /// SDP answer was sent
    AnswerSent {
        /// Session ID
        session_id: String,
        /// Dialog ID
        dialog_id: String,
    },

    /// SDP answer was received
    AnswerReceived {
        /// Session ID
        session_id: String,
        /// Dialog ID
        dialog_id: String,
    },

    /// SDP negotiation is complete
    NegotiationComplete {
        /// Session ID
        session_id: String,
        /// Dialog ID
        dialog_id: String,
    },
}

// Add conversion from SdpEvent to SessionEvent
impl From<SdpEvent> for SessionEvent {
    fn from(event: SdpEvent) -> Self {
        match event {
            SdpEvent::OfferSent { session_id, dialog_id } => {
                let session_id_val = match Uuid::parse_str(&session_id) {
                    Ok(uuid) => SessionId(uuid),
                    Err(_) => SessionId::new(),
                };
                
                SessionEvent::Custom { 
                    session_id: session_id_val,
                    event_type: "sdp:offer_sent".to_string(),
                    data: serde_json::json!({ "dialog_id": dialog_id }),
                }
            },
            SdpEvent::OfferReceived { session_id, dialog_id } => {
                let session_id_val = match Uuid::parse_str(&session_id) {
                    Ok(uuid) => SessionId(uuid),
                    Err(_) => SessionId::new(),
                };
                
                SessionEvent::Custom { 
                    session_id: session_id_val,
                    event_type: "sdp:offer_received".to_string(),
                    data: serde_json::json!({ "dialog_id": dialog_id }),
                }
            },
            SdpEvent::AnswerSent { session_id, dialog_id } => {
                let session_id_val = match Uuid::parse_str(&session_id) {
                    Ok(uuid) => SessionId(uuid),
                    Err(_) => SessionId::new(),
                };
                
                SessionEvent::Custom { 
                    session_id: session_id_val,
                    event_type: "sdp:answer_sent".to_string(),
                    data: serde_json::json!({ "dialog_id": dialog_id }),
                }
            },
            SdpEvent::AnswerReceived { session_id, dialog_id } => {
                let session_id_val = match Uuid::parse_str(&session_id) {
                    Ok(uuid) => SessionId(uuid),
                    Err(_) => SessionId::new(),
                };
                
                SessionEvent::Custom { 
                    session_id: session_id_val,
                    event_type: "sdp:answer_received".to_string(),
                    data: serde_json::json!({ "dialog_id": dialog_id }),
                }
            },
            SdpEvent::NegotiationComplete { session_id, dialog_id } => {
                let session_id_val = match Uuid::parse_str(&session_id) {
                    Ok(uuid) => SessionId(uuid),
                    Err(_) => SessionId::new(),
                };
                
                SessionEvent::Custom { 
                    session_id: session_id_val,
                    event_type: "sdp:negotiation_complete".to_string(),
                    data: serde_json::json!({ "dialog_id": dialog_id }),
                }
            },
        }
    }
} 