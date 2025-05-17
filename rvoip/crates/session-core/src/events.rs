use std::sync::Arc;
use async_trait::async_trait;
use tokio::sync::broadcast;
use uuid::Uuid;
use std::fmt;

use rvoip_sip_core::{Request, Response};
use rvoip_transaction_core::TransactionKey;

use crate::session::{SessionId, SessionState};
use crate::dialog::{DialogId, DialogState};
use crate::sdp::NegotiationState;

/// Event types that can be emitted during session lifecycle
#[derive(Debug, Clone, PartialEq)]
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
        response: Response,
    },
    
    /// Success response received
    SuccessResponse {
        session_id: SessionId,
        response: Response,
    },
    
    /// Failure response received
    FailureResponse {
        session_id: SessionId,
        response: Response,
    },
    
    /// New transaction created
    TransactionCreated {
        session_id: SessionId,
        transaction_id: TransactionKey,
        is_client: bool,
        method: String,
    },
    
    /// Transaction completed
    TransactionCompleted {
        session_id: SessionId,
        transaction_id: TransactionKey,
        success: bool,
    },
    
    /// Custom event type for application-specific events
    Custom {
        session_id: SessionId,
        event_type: String,
        data: serde_json::Value,
    },
    
    /// Session created
    SessionCreated {
        /// Session ID
        session_id: SessionId,
    },
    
    /// Session state changed
    SessionStateChanged {
        /// Session ID
        session_id: SessionId,
        /// Previous state
        previous: SessionState,
        /// New state
        current: SessionState,
    },
    
    /// Dialog created
    DialogCreated {
        /// Session ID
        session_id: SessionId,
        /// Dialog ID
        dialog_id: DialogId,
    },
    
    /// Dialog state changed
    DialogStateChanged {
        /// Session ID
        session_id: SessionId,
        /// Dialog ID
        dialog_id: DialogId,
        /// Previous state
        previous: DialogState,
        /// New state
        current: DialogState,
    },
    
    /// SDP offer sent in a request
    SdpOfferSent {
        /// Session ID
        session_id: SessionId,
        /// Dialog ID
        dialog_id: DialogId,
    },
    
    /// SDP offer received in a request
    SdpOfferReceived {
        /// Session ID
        session_id: SessionId,
        /// Dialog ID
        dialog_id: DialogId,
    },
    
    /// SDP answer sent in a response
    SdpAnswerSent {
        /// Session ID
        session_id: SessionId,
        /// Dialog ID
        dialog_id: DialogId,
    },
    
    /// SDP answer received in a response
    SdpAnswerReceived {
        /// Session ID
        session_id: SessionId,
        /// Dialog ID
        dialog_id: DialogId,
    },
    
    /// SDP negotiation completed
    SdpNegotiationComplete {
        /// Session ID
        session_id: SessionId,
        /// Dialog ID
        dialog_id: DialogId,
    },
    
    /// Dialog recovery started
    DialogRecoveryStarted {
        /// Session ID
        session_id: SessionId,
        /// Dialog ID
        dialog_id: DialogId,
        /// Reason for entering recovery mode
        reason: String,
    },
    
    /// Dialog recovery completed (successfully or not)
    DialogRecoveryCompleted {
        /// Session ID
        session_id: SessionId,
        /// Dialog ID
        dialog_id: DialogId,
        /// Whether recovery was successful
        success: bool,
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

// Unit tests for events module
#[cfg(test)]
mod tests {
    use super::*;
    use std::sync::Arc;
    use std::sync::atomic::{AtomicUsize, Ordering};
    use tokio::time::sleep;
    use std::time::Duration;
    use crate::session::{SessionId, SessionState};
    use crate::dialog::DialogId;

    // Create a test event handler that counts events
    struct TestEventHandler {
        event_count: AtomicUsize,
        created_count: AtomicUsize,
        state_changed_count: AtomicUsize,
        terminated_count: AtomicUsize,
    }

    impl TestEventHandler {
        fn new() -> Self {
            Self {
                event_count: AtomicUsize::new(0),
                created_count: AtomicUsize::new(0),
                state_changed_count: AtomicUsize::new(0),
                terminated_count: AtomicUsize::new(0),
            }
        }
        
        fn event_count(&self) -> usize {
            self.event_count.load(Ordering::SeqCst)
        }
        
        fn created_count(&self) -> usize {
            self.created_count.load(Ordering::SeqCst)
        }
        
        fn state_changed_count(&self) -> usize {
            self.state_changed_count.load(Ordering::SeqCst)
        }
        
        fn terminated_count(&self) -> usize {
            self.terminated_count.load(Ordering::SeqCst)
        }
    }

    #[async_trait]
    impl EventHandler for TestEventHandler {
        async fn handle_event(&self, event: SessionEvent) {
            self.event_count.fetch_add(1, Ordering::SeqCst);
            
            match event {
                SessionEvent::Created { .. } => {
                    self.created_count.fetch_add(1, Ordering::SeqCst);
                },
                SessionEvent::StateChanged { .. } => {
                    self.state_changed_count.fetch_add(1, Ordering::SeqCst);
                },
                SessionEvent::Terminated { .. } => {
                    self.terminated_count.fetch_add(1, Ordering::SeqCst);
                },
                _ => {}
            }
        }
    }

    #[tokio::test]
    async fn test_event_bus_publish_subscribe() {
        // Create event bus
        let event_bus = EventBus::new(10);
        
        // Subscribe to events
        let mut rx = event_bus.subscribe();
        
        // Create a session ID for testing
        let session_id = SessionId::new();
        
        // Publish an event
        event_bus.publish(SessionEvent::Created { 
            session_id: session_id.clone() 
        });
        
        // Try to receive the event with a timeout
        let received = tokio::time::timeout(
            Duration::from_millis(100),
            rx.recv()
        ).await;
        
        // Check if we received the event
        assert!(received.is_ok(), "Failed to receive event within timeout");
        
        if let Ok(event_result) = received {
            match event_result {
                Ok(SessionEvent::Created { session_id: event_session_id }) => {
                    assert_eq!(event_session_id, session_id);
                },
                _ => panic!("Received wrong event type"),
            }
        }
    }

    #[tokio::test]
    async fn test_event_handler_registration() {
        // Create event bus
        let event_bus = EventBus::new(10);
        
        // Create and register handler
        let handler = Arc::new(TestEventHandler::new());
        event_bus.register_handler(handler.clone()).await;
        
        // Create a session ID for testing
        let session_id = SessionId::new();
        let dialog_id = DialogId::new();
        
        // Publish several events
        event_bus.publish(SessionEvent::Created { 
            session_id: session_id.clone() 
        });
        
        event_bus.publish(SessionEvent::StateChanged { 
            session_id: session_id.clone(),
            old_state: SessionState::Initializing,
            new_state: SessionState::Dialing,
        });
        
        event_bus.publish(SessionEvent::DialogUpdated { 
            session_id: session_id.clone(),
            dialog_id,
        });
        
        event_bus.publish(SessionEvent::Terminated { 
            session_id: session_id.clone(),
            reason: "Test termination".to_string(),
        });
        
        // Wait a bit for all events to be processed
        sleep(Duration::from_millis(50)).await;
        
        // Check the counts
        assert_eq!(handler.event_count(), 4);
        assert_eq!(handler.created_count(), 1);
        assert_eq!(handler.state_changed_count(), 1);
        assert_eq!(handler.terminated_count(), 1);
    }

    #[tokio::test]
    async fn test_multiple_subscribers() {
        // Create event bus
        let event_bus = EventBus::new(10);
        
        // Create two subscribers
        let mut rx1 = event_bus.subscribe();
        let mut rx2 = event_bus.subscribe();
        
        // Create a session ID for testing
        let session_id = SessionId::new();
        
        // Publish an event
        event_bus.publish(SessionEvent::Created { 
            session_id: session_id.clone() 
        });
        
        // Try to receive the event on both subscribers
        let received1 = tokio::time::timeout(
            Duration::from_millis(100),
            rx1.recv()
        ).await;
        
        let received2 = tokio::time::timeout(
            Duration::from_millis(100),
            rx2.recv()
        ).await;
        
        // Check if both received the event
        assert!(received1.is_ok(), "Subscriber 1 failed to receive event within timeout");
        assert!(received2.is_ok(), "Subscriber 2 failed to receive event within timeout");
    }

    #[tokio::test]
    async fn test_event_bus_default() {
        // Test the default constructor
        let event_bus = EventBus::default();
        
        // Subscribe to events
        let mut rx = event_bus.subscribe();
        
        // Create a session ID for testing
        let session_id = SessionId::new();
        
        // Publish an event
        event_bus.publish(SessionEvent::Created { 
            session_id: session_id.clone() 
        });
        
        // Verify we can receive it
        let received = tokio::time::timeout(
            Duration::from_millis(100),
            rx.recv()
        ).await;
        
        assert!(received.is_ok(), "Failed to receive event within timeout");
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