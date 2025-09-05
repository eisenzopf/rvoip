//! Signaling Interceptor for handling SIP events before state machine processing
//!
//! This module provides an extensible interception layer that sits between the
//! DialogAdapter and the state machine. It allows for custom routing decisions
//! and automatic session creation for incoming calls.

use async_trait::async_trait;
use tokio::sync::mpsc;
use tracing::{debug, info, warn, error};

use crate::types::{SessionId, DialogId, SessionEvent, IncomingCallInfo};
use crate::session_registry::SessionRegistry;

/// Decision types for handling signaling events
#[derive(Debug, Clone)]
pub enum SignalingDecision {
    /// Accept and process normally
    Accept,
    /// Reject the event (e.g., decline call)
    Reject { reason: String },
    /// Defer decision to application layer
    Defer,
    /// Custom routing (e.g., forward to queue)
    Custom { action: String, data: Option<String> },
}

/// Trait for custom signaling handlers
#[async_trait]
pub trait SignalingHandler: Send + Sync {
    /// Handle an incoming INVITE
    async fn handle_incoming_invite(
        &self,
        from: &str,
        to: &str,
        call_id: &str,
        dialog_id: &DialogId,
    ) -> SignalingDecision;

    /// Handle a response (1xx, 2xx, 3xx, 4xx, 5xx, 6xx)
    async fn handle_response(
        &self,
        status_code: u16,
        dialog_id: &DialogId,
        session_id: Option<&SessionId>,
    ) -> SignalingDecision;

    /// Handle BYE request
    async fn handle_bye(
        &self,
        dialog_id: &DialogId,
        session_id: Option<&SessionId>,
    ) -> SignalingDecision;

    /// Handle CANCEL request
    async fn handle_cancel(
        &self,
        dialog_id: &DialogId,
        session_id: Option<&SessionId>,
    ) -> SignalingDecision;

    /// Handle UPDATE request
    async fn handle_update(
        &self,
        dialog_id: &DialogId,
        session_id: Option<&SessionId>,
    ) -> SignalingDecision;

    /// Handle re-INVITE request
    async fn handle_reinvite(
        &self,
        dialog_id: &DialogId,
        session_id: Option<&SessionId>,
    ) -> SignalingDecision;

    /// Handle REFER request (transfer)
    async fn handle_refer(
        &self,
        dialog_id: &DialogId,
        session_id: Option<&SessionId>,
        refer_to: &str,
    ) -> SignalingDecision;
}

/// Default signaling handler - accepts all events
pub struct DefaultSignalingHandler;

#[async_trait]
impl SignalingHandler for DefaultSignalingHandler {
    async fn handle_incoming_invite(
        &self,
        _from: &str,
        _to: &str,
        _call_id: &str,
        _dialog_id: &DialogId,
    ) -> SignalingDecision {
        SignalingDecision::Accept
    }

    async fn handle_response(
        &self,
        _status_code: u16,
        _dialog_id: &DialogId,
        _session_id: Option<&SessionId>,
    ) -> SignalingDecision {
        SignalingDecision::Accept
    }

    async fn handle_bye(
        &self,
        _dialog_id: &DialogId,
        _session_id: Option<&SessionId>,
    ) -> SignalingDecision {
        SignalingDecision::Accept
    }

    async fn handle_cancel(
        &self,
        _dialog_id: &DialogId,
        _session_id: Option<&SessionId>,
    ) -> SignalingDecision {
        SignalingDecision::Accept
    }

    async fn handle_update(
        &self,
        _dialog_id: &DialogId,
        _session_id: Option<&SessionId>,
    ) -> SignalingDecision {
        SignalingDecision::Accept
    }

    async fn handle_reinvite(
        &self,
        _dialog_id: &DialogId,
        _session_id: Option<&SessionId>,
    ) -> SignalingDecision {
        SignalingDecision::Accept
    }

    async fn handle_refer(
        &self,
        _dialog_id: &DialogId,
        _session_id: Option<&SessionId>,
        _refer_to: &str,
    ) -> SignalingDecision {
        SignalingDecision::Accept
    }
}

/// Signaling interceptor that processes events before the state machine
pub struct SignalingInterceptor {
    /// Custom signaling handler
    handler: Box<dyn SignalingHandler>,
    /// Session registry for ID mappings
    registry: SessionRegistry,
    /// Channel for incoming call notifications
    incoming_call_tx: mpsc::Sender<IncomingCallInfo>,
    /// Channel for session events
    event_tx: mpsc::Sender<(SessionId, SessionEvent)>,
}

impl SignalingInterceptor {
    /// Create a new signaling interceptor
    pub fn new(
        handler: Box<dyn SignalingHandler>,
        registry: SessionRegistry,
        incoming_call_tx: mpsc::Sender<IncomingCallInfo>,
        event_tx: mpsc::Sender<(SessionId, SessionEvent)>,
    ) -> Self {
        Self {
            handler,
            registry,
            incoming_call_tx,
            event_tx,
        }
    }

    /// Create with default handler
    pub fn with_default_handler(
        registry: SessionRegistry,
        incoming_call_tx: mpsc::Sender<IncomingCallInfo>,
        event_tx: mpsc::Sender<(SessionId, SessionEvent)>,
    ) -> Self {
        Self::new(
            Box::new(DefaultSignalingHandler),
            registry,
            incoming_call_tx,
            event_tx,
        )
    }

    /// Handle a signaling event from the DialogAdapter
    pub async fn handle_signaling_event(&self, event: SessionEvent) -> Result<(), String> {
        match &event {
            SessionEvent::IncomingCall { from, to, call_id, dialog_id, .. } => {
                self.handle_incoming_call(from, to, call_id, dialog_id).await
            }
            SessionEvent::CallProgress { dialog_id, status_code, .. } => {
                let session_id = self.registry.get_session_by_dialog(dialog_id);
                let decision = self.handler.handle_response(*status_code, dialog_id, session_id.as_ref()).await;
                
                match decision {
                    SignalingDecision::Accept => {
                        if let Some(session_id) = session_id {
                            self.forward_to_state_machine(session_id, event).await
                        } else {
                            Err("No session found for dialog".to_string())
                        }
                    }
                    SignalingDecision::Reject { reason } => {
                        warn!("Rejected response: {}", reason);
                        Ok(())
                    }
                    _ => Ok(())
                }
            }
            SessionEvent::CallTerminated { dialog_id, .. } => {
                let session_id = self.registry.get_session_by_dialog(dialog_id);
                let decision = self.handler.handle_bye(dialog_id, session_id.as_ref()).await;
                
                match decision {
                    SignalingDecision::Accept => {
                        if let Some(session_id) = session_id {
                            self.forward_to_state_machine(session_id, event).await
                        } else {
                            Err("No session found for dialog".to_string())
                        }
                    }
                    _ => Ok(())
                }
            }
            _ => {
                // For other events, try to find the session and forward
                if let Some(dialog_id) = self.extract_dialog_id(&event) {
                    if let Some(session_id) = self.registry.get_session_by_dialog(&dialog_id) {
                        self.forward_to_state_machine(session_id, event).await
                    } else {
                        Err("No session found for event".to_string())
                    }
                } else {
                    Err("Cannot extract dialog ID from event".to_string())
                }
            }
        }
    }

    /// Handle incoming call
    async fn handle_incoming_call(
        &self,
        from: &str,
        to: &str,
        call_id: &str,
        dialog_id: &DialogId,
    ) -> Result<(), String> {
        let decision = self.handler.handle_incoming_invite(from, to, call_id, dialog_id).await;

        match decision {
            SignalingDecision::Accept => {
                // Create new session for incoming call
                let session_id = SessionId::new();
                
                // Register the dialog-session mapping
                self.registry.map_dialog(session_id.clone(), dialog_id.clone());
                
                // Create incoming call info
                let call_info = IncomingCallInfo {
                    session_id: session_id.clone(),
                    dialog_id: dialog_id.clone(),
                    from: from.to_string(),
                    to: to.to_string(),
                    call_id: call_id.to_string(),
                };
                
                // Send to incoming call channel
                if let Err(e) = self.incoming_call_tx.send(call_info).await {
                    error!("Failed to send incoming call notification: {}", e);
                    return Err(format!("Failed to notify about incoming call: {}", e));
                }
                
                info!("Created session {} for incoming call from {}", session_id.0, from);
                Ok(())
            }
            SignalingDecision::Reject { reason } => {
                info!("Rejecting incoming call from {}: {}", from, reason);
                // TODO: Send SIP rejection response
                Ok(())
            }
            SignalingDecision::Defer => {
                debug!("Deferring decision for incoming call from {}", from);
                // Send to application layer for decision
                let session_id = SessionId::new();
                self.registry.map_dialog(session_id.clone(), dialog_id.clone());
                
                let call_info = IncomingCallInfo {
                    session_id: session_id.clone(),
                    dialog_id: dialog_id.clone(),
                    from: from.to_string(),
                    to: to.to_string(),
                    call_id: call_id.to_string(),
                };
                
                self.incoming_call_tx.send(call_info).await
                    .map_err(|e| format!("Failed to defer call: {}", e))
            }
            SignalingDecision::Custom { action, data } => {
                info!("Custom action for incoming call: {} {:?}", action, data);
                // Handle custom routing (e.g., queue, forward, etc.)
                Ok(())
            }
        }
    }

    /// Forward event to state machine
    async fn forward_to_state_machine(&self, session_id: SessionId, event: SessionEvent) -> Result<(), String> {
        self.event_tx.send((session_id, event)).await
            .map_err(|e| format!("Failed to forward event to state machine: {}", e))
    }

    /// Extract dialog ID from event
    fn extract_dialog_id(&self, event: &SessionEvent) -> Option<DialogId> {
        match event {
            SessionEvent::IncomingCall { dialog_id, .. } |
            SessionEvent::CallProgress { dialog_id, .. } |
            SessionEvent::CallAnswered { dialog_id, .. } |
            SessionEvent::CallTerminated { dialog_id, .. } |
            SessionEvent::CallFailed { dialog_id, .. } => Some(dialog_id.clone()),
            _ => None,
        }
    }

    /// Set a new signaling handler
    pub fn set_handler(&mut self, handler: Box<dyn SignalingHandler>) {
        self.handler = handler;
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use tokio::sync::mpsc;

    struct TestHandler {
        accept_calls: bool,
    }

    #[async_trait]
    impl SignalingHandler for TestHandler {
        async fn handle_incoming_invite(
            &self,
            _from: &str,
            _to: &str,
            _call_id: &str,
            _dialog_id: &DialogId,
        ) -> SignalingDecision {
            if self.accept_calls {
                SignalingDecision::Accept
            } else {
                SignalingDecision::Reject { 
                    reason: "Test rejection".to_string() 
                }
            }
        }

        async fn handle_response(
            &self,
            _status_code: u16,
            _dialog_id: &DialogId,
            _session_id: Option<&SessionId>,
        ) -> SignalingDecision {
            SignalingDecision::Accept
        }

        async fn handle_bye(
            &self,
            _dialog_id: &DialogId,
            _session_id: Option<&SessionId>,
        ) -> SignalingDecision {
            SignalingDecision::Accept
        }

        async fn handle_cancel(
            &self,
            _dialog_id: &DialogId,
            _session_id: Option<&SessionId>,
        ) -> SignalingDecision {
            SignalingDecision::Accept
        }

        async fn handle_update(
            &self,
            _dialog_id: &DialogId,
            _session_id: Option<&SessionId>,
        ) -> SignalingDecision {
            SignalingDecision::Accept
        }

        async fn handle_reinvite(
            &self,
            _dialog_id: &DialogId,
            _session_id: Option<&SessionId>,
        ) -> SignalingDecision {
            SignalingDecision::Accept
        }

        async fn handle_refer(
            &self,
            _dialog_id: &DialogId,
            _session_id: Option<&SessionId>,
            _refer_to: &str,
        ) -> SignalingDecision {
            SignalingDecision::Accept
        }
    }

    #[tokio::test]
    async fn test_incoming_call_accepted() {
        let registry = SessionRegistry::new();
        let (call_tx, mut call_rx) = mpsc::channel(10);
        let (event_tx, _event_rx) = mpsc::channel(10);
        
        let handler = Box::new(TestHandler { accept_calls: true });
        let interceptor = SignalingInterceptor::new(handler, registry, call_tx, event_tx);
        
        let dialog_id = DialogId::new();
        let event = SessionEvent::IncomingCall {
            from: "alice@example.com".to_string(),
            to: "bob@example.com".to_string(),
            call_id: "call123".to_string(),
            dialog_id: dialog_id.clone(),
            sdp: None,
        };
        
        interceptor.handle_signaling_event(event).await.unwrap();
        
        let call_info = call_rx.recv().await.unwrap();
        assert_eq!(call_info.from, "alice@example.com");
        assert_eq!(call_info.dialog_id, dialog_id);
    }

    #[tokio::test]
    async fn test_incoming_call_rejected() {
        let registry = SessionRegistry::new();
        let (call_tx, mut call_rx) = mpsc::channel(10);
        let (event_tx, _event_rx) = mpsc::channel(10);
        
        let handler = Box::new(TestHandler { accept_calls: false });
        let interceptor = SignalingInterceptor::new(handler, registry, call_tx, event_tx);
        
        let event = SessionEvent::IncomingCall {
            from: "alice@example.com".to_string(),
            to: "bob@example.com".to_string(),
            call_id: "call123".to_string(),
            dialog_id: DialogId::new(),
            sdp: None,
        };
        
        interceptor.handle_signaling_event(event).await.unwrap();
        
        // Should not receive call notification when rejected
        assert!(call_rx.try_recv().is_err());
    }
}