//! Dialog Bridge (parallel to MediaBridge)
//!
//! Event integration between dialog-core and session systems.
//! Handles bidirectional event flow and state synchronization.

use std::sync::Arc;
use tokio::sync::mpsc;
use rvoip_dialog_core::events::SessionCoordinationEvent;
use crate::api::types::SessionId;
use crate::manager::events::SessionEvent;
use crate::dialog::{DialogError, DialogResult, DialogSessionMap};

/// Dialog bridge for session-dialog event integration
/// (parallel to MediaBridge)
pub struct DialogBridge {
    session_events_tx: mpsc::Sender<SessionEvent>,
    dialog_events_tx: mpsc::Sender<SessionCoordinationEvent>,
    dialog_to_session: Arc<dashmap::DashMap<rvoip_dialog_core::DialogId, SessionId>>,
}

impl DialogBridge {
    /// Create a new dialog bridge
    pub fn new(
        session_events_tx: mpsc::Sender<SessionEvent>,
        dialog_events_tx: mpsc::Sender<SessionCoordinationEvent>,
        dialog_to_session: Arc<dashmap::DashMap<rvoip_dialog_core::DialogId, SessionId>>,
    ) -> Self {
        Self {
            session_events_tx,
            dialog_events_tx,
            dialog_to_session,
        }
    }
    
    /// Bridge dialog event to session event
    pub async fn bridge_dialog_to_session(
        &self,
        dialog_event: SessionCoordinationEvent,
    ) -> DialogResult<()> {
        // Convert dialog event to session event
        let session_event = self.convert_dialog_to_session_event(dialog_event)?;
        
        // Send to session event processor
        self.session_events_tx
            .send(session_event)
            .await
            .map_err(|e| DialogError::Coordination {
                message: format!("Failed to send session event: {}", e),
            })?;
            
        Ok(())
    }
    
    /// Bridge session event to dialog event (if needed)
    pub async fn bridge_session_to_dialog(
        &self,
        session_event: SessionEvent,
    ) -> DialogResult<()> {
        // Convert session event to dialog coordination event if relevant
        if let Some(dialog_event) = self.convert_session_to_dialog_event(session_event)? {
            self.dialog_events_tx
                .send(dialog_event)
                .await
                .map_err(|e| DialogError::Coordination {
                    message: format!("Failed to send dialog event: {}", e),
                })?;
        }
        
        Ok(())
    }
    
    /// Convert dialog coordination event to session event
    fn convert_dialog_to_session_event(
        &self,
        dialog_event: SessionCoordinationEvent,
    ) -> DialogResult<SessionEvent> {
        match dialog_event {
            SessionCoordinationEvent::IncomingCall { dialog_id, .. } => {
                // Extract session ID from dialog mapping
                let session_id = self.dialog_to_session
                    .get(&dialog_id)
                    .map(|entry| entry.value().clone())
                    .ok_or_else(|| DialogError::SessionNotFound {
                        session_id: format!("dialog:{}", dialog_id),
                    })?;
                    
                Ok(SessionEvent::SessionCreated {
                    session_id,
                    from: "unknown".to_string(), // Will be extracted properly in coordinator
                    to: "unknown".to_string(),
                    call_state: crate::api::types::CallState::Ringing,
                })
            }
            
            SessionCoordinationEvent::CallAnswered { dialog_id, .. } => {
                let session_id = self.dialog_to_session
                    .get(&dialog_id)
                    .map(|entry| entry.value().clone())
                    .ok_or_else(|| DialogError::SessionNotFound {
                        session_id: format!("dialog:{}", dialog_id),
                    })?;
                    
                Ok(SessionEvent::StateChanged {
                    session_id,
                    old_state: crate::api::types::CallState::Ringing,
                    new_state: crate::api::types::CallState::Active,
                })
            }
            
            SessionCoordinationEvent::ResponseReceived { dialog_id, response, .. } => {
                let session_id = self.dialog_to_session
                    .get(&dialog_id)
                    .map(|entry| entry.value().clone())
                    .ok_or_else(|| DialogError::SessionNotFound {
                        session_id: format!("dialog:{}", dialog_id),
                    })?;
                    
                // Convert based on response status
                let call_state = match response.status_code() {
                    200 => crate::api::types::CallState::Active,
                    180 | 183 => crate::api::types::CallState::Ringing,
                    487 => crate::api::types::CallState::Cancelled,
                    code if code >= 400 => crate::api::types::CallState::Failed(
                        format!("SIP {}: {}", code, response.reason_phrase())
                    ),
                    _ => return Err(DialogError::SipProcessing {
                        message: format!("Unhandled response code: {}", response.status_code()),
                    }),
                };
                
                Ok(SessionEvent::StateChanged {
                    session_id,
                    old_state: crate::api::types::CallState::Initiating, // Simplified
                    new_state: call_state,
                })
            }
            
            // Handle other dialog events that we don't currently bridge
            _ => {
                tracing::debug!("Unhandled dialog coordination event in bridge: {:?}", dialog_event);
                Err(DialogError::SipProcessing {
                    message: "Dialog event not supported for bridging".to_string(),
                })
            }
        }
    }
    
    /// Convert session event to dialog coordination event (if applicable)
    fn convert_session_to_dialog_event(
        &self,
        _session_event: SessionEvent,
    ) -> DialogResult<Option<SessionCoordinationEvent>> {
        // Most session events don't need to be bridged back to dialog-core
        // This is mainly for future extensibility
        Ok(None)
    }
    
    /// Get session ID for dialog ID
    pub fn get_session_for_dialog(&self, dialog_id: &rvoip_dialog_core::DialogId) -> Option<SessionId> {
        self.dialog_to_session
            .get(dialog_id)
            .map(|entry| entry.value().clone())
    }
    
    /// Add dialog-to-session mapping
    pub fn map_dialog_to_session(
        &self,
        dialog_id: rvoip_dialog_core::DialogId,
        session_id: SessionId,
    ) {
        self.dialog_to_session.insert(dialog_id, session_id);
    }
    
    /// Remove dialog-to-session mapping
    pub fn unmap_dialog(&self, dialog_id: &rvoip_dialog_core::DialogId) {
        self.dialog_to_session.remove(dialog_id);
    }
}

impl std::fmt::Debug for DialogBridge {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DialogBridge")
            .field("mapped_dialogs", &self.dialog_to_session.len())
            .finish()
    }
} 