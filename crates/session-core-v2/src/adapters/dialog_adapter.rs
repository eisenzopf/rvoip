//! Simplified Dialog Adapter for session-core-v2
//!
//! Thin translation layer between dialog-core and state machine.
//! Focuses only on essential dialog operations and events.

use std::sync::Arc;
use std::net::SocketAddr;
use tokio::sync::mpsc;
use dashmap::DashMap;
use rvoip_dialog_core::{
    api::unified::UnifiedDialogApi,
    events::{SessionCoordinationEvent, DialogEvent},
    DialogId,
    transaction::TransactionKey,
};
use rvoip_sip_core::{Request, Response, StatusCode};
use crate::state_table::types::{SessionId, EventType};
use crate::errors::{Result, SessionError};
use crate::session_store::SessionStore;

/// Minimal dialog adapter - just translates between dialog-core and state machine
pub struct DialogAdapter {
    /// Dialog-core unified API
    dialog_api: Arc<UnifiedDialogApi>,
    
    /// Channel to send events to state machine
    event_tx: mpsc::Sender<(SessionId, EventType)>,
    
    /// Session store for updating IDs
    store: Arc<SessionStore>,
    
    /// Simple mapping of session IDs to dialog IDs
    session_to_dialog: Arc<DashMap<SessionId, DialogId>>,
    dialog_to_session: Arc<DashMap<DialogId, SessionId>>,
    
    /// Store Call-ID to session mapping for correlation
    callid_to_session: Arc<DashMap<String, SessionId>>,
    
    /// Store incoming request data for UAS responses
    incoming_requests: Arc<DashMap<SessionId, (Request, TransactionKey, SocketAddr)>>,
    
    /// Store outgoing INVITE transaction IDs for UAC ACK sending
    outgoing_invite_tx: Arc<DashMap<SessionId, TransactionKey>>,
}

impl DialogAdapter {
    pub fn new(
        dialog_api: Arc<UnifiedDialogApi>,
        event_tx: mpsc::Sender<(SessionId, EventType)>,
        store: Arc<SessionStore>,
    ) -> Self {
        Self {
            dialog_api,
            event_tx,
            store,
            session_to_dialog: Arc::new(DashMap::new()),
            dialog_to_session: Arc::new(DashMap::new()),
            callid_to_session: Arc::new(DashMap::new()),
            incoming_requests: Arc::new(DashMap::new()),
            outgoing_invite_tx: Arc::new(DashMap::new()),
        }
    }
    
    // ===== Outbound Actions (from state machine) =====
    
    /// Send INVITE for UAC
    pub async fn send_invite(
        &self,
        session_id: &SessionId,
        from: &str,
        to: &str,
        sdp: Option<String>,
    ) -> Result<()> {
        // Use make_call_with_id to control the Call-ID
        let call_id = format!("{}@session-core", session_id.0);
        
        let call_handle = self.dialog_api
            .make_call_with_id(from, to, sdp, Some(call_id.clone()))
            .await
            .map_err(|e| SessionError::DialogError(format!("Failed to make call: {}", e)))?;
        
        let dialog_id = call_handle.call_id().clone();
        
        // Store mappings
        self.session_to_dialog.insert(session_id.clone(), dialog_id.clone());
        self.dialog_to_session.insert(dialog_id.clone(), session_id.clone());
        self.callid_to_session.insert(call_id.clone(), session_id.clone());
        
        // Update session store with dialog and call IDs
        if let Ok(mut session) = self.store.get_session(session_id).await {
            session.dialog_id = Some(dialog_id.to_string());
            session.call_id = Some(call_id);
            let _ = self.store.update_session(session).await;
        }
        
        Ok(())
    }
    
    /// Send response (for UAS)
    pub async fn send_response(
        &self,
        session_id: &SessionId,
        code: u16,
        sdp: Option<String>,
    ) -> Result<()> {
        // Get stored request data
        let (request, transaction_id, source) = self.incoming_requests
            .get(session_id)
            .ok_or_else(|| SessionError::SessionNotFound(format!("No incoming request for session {}", session_id.0)))?
            .clone();
        
        // Build response using transaction ID
        let mut response = self.dialog_api
            .build_response(&transaction_id, StatusCode::from_u16(code).unwrap_or(StatusCode::Ok), sdp.clone())
            .await
            .map_err(|e| SessionError::DialogError(format!("Failed to build response: {}", e)))?;
        
        // Send the response
        self.dialog_api
            .send_response(&transaction_id, response)
            .await
            .map_err(|e| SessionError::DialogError(format!("Failed to send response: {}", e)))?;
        
        // Clean up stored request after successful response
        if code >= 200 {
            self.incoming_requests.remove(session_id);
        }
        
        Ok(())
    }
    
    /// Send ACK (for UAC after 200 OK)
    pub async fn send_ack(&self, session_id: &SessionId, response: &Response) -> Result<()> {
        // Get the dialog ID for this session
        let dialog_id = self.session_to_dialog.get(session_id)
            .ok_or_else(|| SessionError::SessionNotFound(session_id.0.clone()))?
            .clone();
        
        // Check if we have the original INVITE transaction ID stored
        if let Some(tx_id) = self.outgoing_invite_tx.get(session_id) {
            // Use the proper ACK method with transaction ID
            self.dialog_api
                .send_ack_for_2xx_response(&dialog_id, &tx_id, response)
                .await
                .map_err(|e| SessionError::DialogError(format!("Failed to send ACK: {}", e)))?;
            
            // Clean up the stored transaction ID after successful ACK
            self.outgoing_invite_tx.remove(session_id);
        } else {
            // Fallback: Try to send ACK without transaction ID (may not work properly)
            tracing::warn!("No transaction ID stored for session {}, ACK may fail", session_id.0);
            // The dialog-core API doesn't have a direct send_ack without transaction ID
            // so we'll need to handle this case differently in production
        }
        
        Ok(())
    }
    
    /// Send BYE to terminate call
    pub async fn send_bye(&self, session_id: &SessionId) -> Result<()> {
        let dialog_id = self.session_to_dialog.get(session_id)
            .ok_or_else(|| SessionError::SessionNotFound(session_id.0.clone()))?
            .clone();
        
        self.dialog_api
            .send_bye(&dialog_id)
            .await
            .map_err(|e| SessionError::DialogError(format!("Failed to send BYE: {}", e)))?;
        
        Ok(())
    }
    
    /// Send CANCEL to cancel pending INVITE
    pub async fn send_cancel(&self, session_id: &SessionId) -> Result<()> {
        let dialog_id = self.session_to_dialog.get(session_id)
            .ok_or_else(|| SessionError::SessionNotFound(session_id.0.clone()))?
            .clone();
        
        self.dialog_api
            .send_cancel(&dialog_id)
            .await
            .map_err(|e| SessionError::DialogError(format!("Failed to send CANCEL: {}", e)))?;
        
        Ok(())
    }
    
    /// Send REFER for blind transfer
    pub async fn send_refer(&self, session_id: &SessionId, refer_to: &str) -> Result<()> {
        let dialog_id = self.session_to_dialog.get(session_id)
            .ok_or_else(|| SessionError::SessionNotFound(session_id.0.clone()))?
            .clone();
        
        // Send REFER through dialog API
        self.dialog_api
            .send_refer(&dialog_id, refer_to.to_string(), None)
            .await
            .map_err(|e| SessionError::DialogError(format!("Failed to send REFER: {}", e)))?;
        
        tracing::info!("Sent REFER to {} for session {}", refer_to, session_id.0);
        Ok(())
    }
    
    /// Send re-INVITE (for hold/resume)
    pub async fn send_reinvite(&self, session_id: &SessionId, sdp: String) -> Result<()> {
        let dialog_id = self.session_to_dialog.get(session_id)
            .ok_or_else(|| SessionError::SessionNotFound(session_id.0.clone()))?
            .clone();
        
        // Use UPDATE method for re-INVITE
        self.dialog_api
            .send_update(&dialog_id, Some(sdp))
            .await
            .map_err(|e| SessionError::DialogError(format!("Failed to send re-INVITE: {}", e)))?;
        
        Ok(())
    }
    
    /// Clean up all mappings and resources for a session
    pub async fn cleanup_session(&self, session_id: &SessionId) -> Result<()> {
        // Remove from all mappings
        if let Some(dialog_id) = self.session_to_dialog.remove(session_id) {
            self.dialog_to_session.remove(&dialog_id.1);
        }
        
        if let Some(entry) = self.callid_to_session.iter()
            .find(|entry| entry.value() == session_id) {
            let call_id = entry.key().clone();
            drop(entry); // Release the reference before removing
            self.callid_to_session.remove(&call_id);
        }
        
        self.incoming_requests.remove(session_id);
        self.outgoing_invite_tx.remove(session_id);
        
        tracing::debug!("Cleaned up dialog adapter mappings for session {}", session_id.0);
        Ok(())
    }
    
    // ===== Inbound Events (from dialog-core) =====
    
    /// Start listening for dialog events
    pub async fn start_event_loop(&self) -> Result<()> {
        // Set up channels for session coordination events
        let (session_tx, mut session_rx) = mpsc::channel(1000);
        self.dialog_api
            .set_session_coordinator(session_tx)
            .await
            .map_err(|e| SessionError::DialogError(format!("Failed to set session coordinator: {}", e)))?;
        
        // Set up channels for dialog events
        let (dialog_tx, mut dialog_rx) = mpsc::channel(1000);
        self.dialog_api
            .set_dialog_event_sender(dialog_tx)
            .await
            .map_err(|e| SessionError::DialogError(format!("Failed to set dialog event sender: {}", e)))?;
        
        // Start the dialog API
        self.dialog_api
            .start()
            .await
            .map_err(|e| SessionError::DialogError(format!("Failed to start dialog API: {}", e)))?;
        
        let adapter = self.clone();
        let adapter2 = self.clone();
        
        // Spawn task to handle session coordination events
        tokio::spawn(async move {
            while let Some(event) = session_rx.recv().await {
                if let Err(e) = adapter.handle_session_event(event).await {
                    tracing::error!("Error handling session event: {}", e);
                }
            }
        });
        
        // Spawn task to handle dialog events
        tokio::spawn(async move {
            while let Some(event) = dialog_rx.recv().await {
                if let Err(e) = adapter2.handle_dialog_event(event).await {
                    tracing::error!("Error handling dialog event: {}", e);
                }
            }
        });
        
        Ok(())
    }
    
    /// Handle session coordination events from dialog-core
    async fn handle_session_event(&self, event: SessionCoordinationEvent) -> Result<()> {
        match event {
            SessionCoordinationEvent::IncomingCall { dialog_id, transaction_id, request, source } => {
                // Extract Call-ID for correlation
                let call_id = request.call_id()
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| format!("unknown-{}", uuid::Uuid::new_v4()));
                let session_id = SessionId::new();
                
                // Store mappings
                self.dialog_to_session.insert(dialog_id.clone(), session_id.clone());
                self.session_to_dialog.insert(session_id.clone(), dialog_id);
                self.callid_to_session.insert(call_id, session_id.clone());
                
                // Store request data for UAS responses
                self.incoming_requests.insert(session_id.clone(), (request.clone(), transaction_id, source));
                
                // Extract SDP if present
                let sdp = if !request.body().is_empty() {
                    String::from_utf8(request.body().to_vec()).ok()
                } else {
                    None
                };
                
                // Send IncomingCall event to state machine
                self.event_tx.send((
                    session_id,
                    EventType::IncomingCall {
                        from: request.from()
                            .map(|f| f.to_string())
                            .unwrap_or_else(|| "anonymous".to_string()),
                        sdp,
                    }
                )).await.map_err(|e| SessionError::InternalError(format!("Failed to send event: {}", e)))?;
            }
            
            SessionCoordinationEvent::ResponseReceived { dialog_id, response, .. } => {
                if let Some(session_id) = self.dialog_to_session.get(&dialog_id) {
                    // Translate response code to event
                    let status_code = response.status_code();
                    let event = match status_code {
                        100 => return Ok(()), // Ignore 100 Trying
                        180 => EventType::Dialog180Ringing,
                        200 => {
                            // Store the 200 OK response for ACK
                            if let Ok(mut session) = self.store.get_session(&session_id).await {
                                // Serialize the response for storage
                                if let Ok(serialized) = bincode::serialize(&response) {
                                    session.last_200_ok = Some(serialized);
                                    
                                    // Also extract and store SDP if present
                                    if !response.body().is_empty() {
                                        if let Some(sdp) = String::from_utf8(response.body().to_vec()).ok() {
                                            session.remote_sdp = Some(sdp);
                                            tracing::debug!("Stored 200 OK with SDP for session {}", session_id.0);
                                        }
                                    }
                                    
                                    let _ = self.store.update_session(session).await;
                                }
                            }
                            EventType::Dialog200OK
                        }
                        code if code >= 400 => {
                            EventType::DialogError(format!("Call failed: {}", code))
                        }
                        _ => return Ok(()), // Ignore other responses
                    };
                    
                    self.event_tx.send((session_id.clone(), event))
                        .await
                        .map_err(|e| SessionError::InternalError(format!("Failed to send event: {}", e)))?;
                }
            }
            
            SessionCoordinationEvent::CallTerminating { dialog_id, reason } => {
                if let Some(session_id) = self.dialog_to_session.get(&dialog_id) {
                    self.event_tx.send((
                        session_id.clone(),
                        EventType::DialogBYE
                    )).await.map_err(|e| SessionError::InternalError(format!("Failed to send event: {}", e)))?;
                }
            }
            
            _ => {
                // Ignore other events for now
            }
        }
        
        Ok(())
    }
    
    /// Handle dialog events from dialog-core
    async fn handle_dialog_event(&self, event: DialogEvent) -> Result<()> {
        match event {
            DialogEvent::Created { dialog_id } => {
                tracing::debug!("Dialog created: {:?}", dialog_id);
            }
            
            DialogEvent::StateChanged { dialog_id, old_state, new_state } => {
                tracing::debug!("Dialog state changed: {:?} from {:?} to {:?}", dialog_id, old_state, new_state);
                // Note: ACK received will be handled through SessionCoordinationEvent
            }
            
            DialogEvent::Terminated { dialog_id, reason } => {
                if let Some(session_id) = self.dialog_to_session.get(&dialog_id) {
                    tracing::debug!("Dialog terminated: {:?}, reason: {}", dialog_id, reason);
                    // BYE will be handled through SessionCoordinationEvent::CallTerminating
                }
            }
            
            _ => {
                // Ignore other events for now
            }
        }
        
        Ok(())
    }
}

impl Clone for DialogAdapter {
    fn clone(&self) -> Self {
        Self {
            dialog_api: self.dialog_api.clone(),
            event_tx: self.event_tx.clone(),
            store: self.store.clone(),
            session_to_dialog: self.session_to_dialog.clone(),
            dialog_to_session: self.dialog_to_session.clone(),
            callid_to_session: self.callid_to_session.clone(),
            incoming_requests: self.incoming_requests.clone(),
            outgoing_invite_tx: self.outgoing_invite_tx.clone(),
        }
    }
}