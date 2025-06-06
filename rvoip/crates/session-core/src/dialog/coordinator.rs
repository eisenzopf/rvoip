//! Session Dialog Coordinator (parallel to SessionMediaCoordinator)
//!
//! Manages the coordination between session-core and dialog-core,
//! handling event bridging and lifecycle management.

use std::sync::Arc;
use tokio::sync::mpsc;
use rvoip_dialog_core::{
    api::unified::UnifiedDialogApi,
    events::SessionCoordinationEvent,
    DialogId,
};
use crate::api::{
    types::{SessionId, CallSession, IncomingCall, CallDecision, CallState},
    handlers::CallHandler,
};
use crate::manager::{registry::SessionRegistry, events::SessionEvent};
use crate::dialog::{DialogError, DialogResult};

/// Session dialog coordinator for automatic dialog lifecycle management
/// (parallel to SessionMediaCoordinator)
pub struct SessionDialogCoordinator {
    dialog_api: Arc<UnifiedDialogApi>,
    registry: Arc<SessionRegistry>,
    handler: Option<Arc<dyn CallHandler>>,
    session_events_tx: mpsc::Sender<SessionEvent>,
    dialog_to_session: Arc<dashmap::DashMap<DialogId, SessionId>>,
}

impl SessionDialogCoordinator {
    /// Create a new session dialog coordinator
    pub fn new(
        dialog_api: Arc<UnifiedDialogApi>,
        registry: Arc<SessionRegistry>,
        handler: Option<Arc<dyn CallHandler>>,
        session_events_tx: mpsc::Sender<SessionEvent>,
        dialog_to_session: Arc<dashmap::DashMap<DialogId, SessionId>>,
    ) -> Self {
        Self {
            dialog_api,
            registry,
            handler,
            session_events_tx,
            dialog_to_session,
        }
    }
    
    /// Initialize session coordination with dialog-core
    pub async fn initialize(&self, session_events_tx: mpsc::Sender<SessionCoordinationEvent>) -> DialogResult<()> {
        // Set up session coordination with dialog-core
        println!("🔗 SETUP: Setting up session coordination with dialog-core");
        self.dialog_api
            .set_session_coordinator(session_events_tx)
            .await
            .map_err(|e| DialogError::Coordination {
                message: format!("Failed to set session coordinator: {}", e),
            })?;
        println!("✅ SETUP: Session coordination setup complete");
        
        Ok(())
    }
    
    /// Start the session coordination event loop
    pub async fn start_event_loop(
        &self,
        mut session_events_rx: mpsc::Receiver<SessionCoordinationEvent>,
    ) -> DialogResult<()> {
        // Spawn task to handle session coordination events
        println!("🎬 SPAWN: Starting session coordination event loop");
        let coordinator = self.clone();
        tokio::spawn(async move {
            println!("📡 EVENT LOOP: Session coordination event loop started");
            while let Some(event) = session_events_rx.recv().await {
                println!("📨 EVENT LOOP: Received session coordination event in background task");
                if let Err(e) = coordinator.handle_session_coordination_event(event).await {
                    tracing::error!("Error handling session coordination event: {}", e);
                }
            }
            println!("🏁 EVENT LOOP: Session coordination event loop ended");
        });
        
        Ok(())
    }
    
    /// Handle session coordination events from dialog-core
    pub async fn handle_session_coordination_event(&self, event: SessionCoordinationEvent) -> DialogResult<()> {
        println!("🎪 SESSION COORDINATION: Received event: {:?}", event);
        match event {
            SessionCoordinationEvent::IncomingCall { dialog_id, transaction_id, request, source } => {
                self.handle_incoming_call(dialog_id, transaction_id, request, source).await?;
            }
            
            SessionCoordinationEvent::ResponseReceived { dialog_id, response, transaction_id } => {
                self.handle_response_received(dialog_id, response, transaction_id).await?;
            }
            
            SessionCoordinationEvent::CallAnswered { dialog_id, session_answer } => {
                self.handle_call_answered(dialog_id, session_answer).await?;
            }
            
            SessionCoordinationEvent::CallTerminated { dialog_id, reason } => {
                self.handle_call_terminated(dialog_id, reason).await?;
            }
            
            SessionCoordinationEvent::RegistrationRequest { transaction_id, from_uri, contact_uri, expires } => {
                self.handle_registration_request(transaction_id, from_uri.to_string(), contact_uri.to_string(), expires).await?;
            }
            
            _ => {
                tracing::debug!("Unhandled session coordination event: {:?}", event);
                // TODO: Handle other events as needed
            }
        }
        
        Ok(())
    }
    
    /// Handle incoming call coordination event
    async fn handle_incoming_call(
        &self,
        dialog_id: DialogId,
        _transaction_id: rvoip_dialog_core::TransactionKey,
        request: rvoip_sip_core::Request,
        source: std::net::SocketAddr,
    ) -> DialogResult<()> {
        // Extract From and To headers - simplified for now
        let from_uri = format!("sip:from@{}", source.ip());
        let to_uri = "sip:to@local".to_string();
        
        tracing::info!("Incoming call from dialog {}: {} -> {}", 
            dialog_id, from_uri, to_uri
        );
        
        // Create a new session for the incoming call
        let session_id = SessionId::new();
        self.dialog_to_session.insert(dialog_id.clone(), session_id.clone());
        
        let call_session = CallSession {
            id: session_id.clone(),
            from: from_uri.clone(),
            to: to_uri.clone(),
            state: CallState::Ringing,
            started_at: Some(std::time::Instant::now()),
            manager: Arc::new(self.create_mock_manager()), // TODO: Fix this circular dependency
        };
        
        self.registry.register_session(session_id.clone(), call_session.clone()).await
            .map_err(|e| DialogError::Coordination {
                message: format!("Failed to register session: {}", e),
            })?;
        
        // Send session created event
        self.send_session_event(SessionEvent::SessionCreated {
            session_id: session_id.clone(),
            from: call_session.from.clone(),
            to: call_session.to.clone(),
            call_state: call_session.state.clone(),
        }).await?;
        
        // Handle the call with the configured handler
        if let Some(handler) = &self.handler {
            let incoming_call = IncomingCall {
                id: session_id.clone(),
                from: from_uri,
                to: to_uri,
                sdp: Some(String::from_utf8_lossy(request.body()).to_string()).filter(|s| !s.is_empty()),
                headers: std::collections::HashMap::new(), // TODO: Extract relevant headers
                received_at: std::time::Instant::now(),
            };
            
            let decision = handler.on_incoming_call(incoming_call).await;
            self.process_call_decision(session_id, dialog_id, decision).await?;
        }
        
        Ok(())
    }
    
    /// Process call decision from handler
    async fn process_call_decision(
        &self,
        session_id: SessionId,
        dialog_id: DialogId,
        decision: CallDecision,
    ) -> DialogResult<()> {
        match decision {
            CallDecision::Accept => {
                // Get the call handle for this dialog and answer it
                if let Ok(call_handle) = self.dialog_api.get_call_handle(&dialog_id).await {
                    if let Err(e) = call_handle.answer(None).await {
                        tracing::error!("Failed to answer incoming call for session {}: {}", session_id, e);
                        self.update_session_state(session_id, CallState::Failed(format!("Answer failed: {}", e))).await?;
                    } else {
                        tracing::info!("Successfully answered incoming call for session {}", session_id);
                    }
                } else {
                    tracing::error!("Failed to get call handle for dialog {} to answer call", dialog_id);
                }
            }
            
            CallDecision::Reject(reason) => {
                tracing::info!("Rejecting incoming call for session {}: {}", session_id, reason);
                
                // Get the call handle and reject it
                if let Ok(call_handle) = self.dialog_api.get_call_handle(&dialog_id).await {
                    if let Err(e) = call_handle.reject(rvoip_sip_core::StatusCode::BusyHere, Some(reason.clone())).await {
                        tracing::error!("Failed to reject incoming call for session {}: {}", session_id, e);
                    }
                }
                
                // Update session state to failed/rejected
                self.update_session_state(session_id, CallState::Failed(reason)).await?;
            }
            
            CallDecision::Defer => {
                tracing::info!("Deferring incoming call for session {} (e.g., added to queue)", session_id);
                // Call remains in Ringing state for manual acceptance later
            }
            
            CallDecision::Forward(target) => {
                tracing::info!("Forwarding incoming call for session {} to {}", session_id, target);
                // TODO: Implement call forwarding via dialog-core
                // For now, treat as rejection
                if let Ok(call_handle) = self.dialog_api.get_call_handle(&dialog_id).await {
                    if let Err(e) = call_handle.reject(
                        rvoip_sip_core::StatusCode::MovedTemporarily, 
                        Some(format!("Forwarded to {}", target))
                    ).await {
                        tracing::error!("Failed to forward incoming call for session {}: {}", session_id, e);
                    }
                }
            }
        }
        
        Ok(())
    }
    
    /// Handle response received coordination event
    async fn handle_response_received(
        &self,
        dialog_id: DialogId,
        response: rvoip_sip_core::Response,
        transaction_id: rvoip_dialog_core::TransactionKey,
    ) -> DialogResult<()> {
        println!("🎯 SESSION COORDINATION: Received response {} for dialog {}", response.status_code(), dialog_id);
        
        // Check if this is a 200 OK response to an INVITE that needs an ACK
        if response.status_code() == 200 && transaction_id.to_string().contains("INVITE") && transaction_id.to_string().contains("client") {
            println!("🚀 SESSION COORDINATION: This is a 200 OK to INVITE - sending automatic ACK");
            
            // Send ACK for 2xx response using the proper dialog-core API
            match self.dialog_api.send_ack_for_2xx_response(&dialog_id, &transaction_id, &response).await {
                Ok(_) => {
                    println!("✅ SESSION COORDINATION: Successfully sent ACK for 200 OK response");
                    tracing::info!("ACK sent successfully for dialog {} transaction {}", dialog_id, transaction_id);
                }
                Err(e) => {
                    println!("❌ SESSION COORDINATION: Failed to send ACK: {}", e);
                    tracing::error!("Failed to send ACK for dialog {} transaction {}: {}", dialog_id, transaction_id, e);
                }
            }
        }
        
        // Continue with other response processing...
        tracing::debug!("Response {} received for dialog {}", response.status_code(), dialog_id);
        
        Ok(())
    }
    
    /// Handle call answered coordination event
    async fn handle_call_answered(
        &self,
        dialog_id: DialogId,
        _session_answer: String,
    ) -> DialogResult<()> {
        if let Some(session_id_ref) = self.dialog_to_session.get(&dialog_id) {
            let session_id = session_id_ref.value().clone();
            tracing::info!("Call answered for session {}: {}", session_id, dialog_id);
            
            // Update call state to Active
            self.update_session_state(session_id, CallState::Active).await?;
        }
        
        Ok(())
    }
    
    /// Handle call terminated coordination event
    async fn handle_call_terminated(
        &self,
        dialog_id: DialogId,
        reason: String,
    ) -> DialogResult<()> {
        if let Some((_, session_id)) = self.dialog_to_session.remove(&dialog_id) {
            tracing::info!("Call terminated for session {}: {} - {}", session_id, dialog_id, reason);
            
            // Send session terminated event
            self.send_session_event(SessionEvent::SessionTerminated {
                session_id: session_id.clone(),
                reason: reason.clone(),
            }).await?;
            
            self.registry.unregister_session(&session_id).await
                .map_err(|e| DialogError::Coordination {
                    message: format!("Failed to unregister session: {}", e),
                })?;
        }
        
        Ok(())
    }
    
    /// Handle registration request coordination event
    async fn handle_registration_request(
        &self,
        _transaction_id: rvoip_dialog_core::TransactionKey,
        from_uri: String,
        contact_uri: String,
        expires: u32,
    ) -> DialogResult<()> {
        tracing::info!("Registration request: {} -> {} (expires: {})", from_uri, contact_uri, expires);
        // Handle registration - for now just log it
        // In a real implementation, this would update a registration database
        Ok(())
    }
    
    /// Update session state and send event
    async fn update_session_state(&self, session_id: SessionId, new_state: CallState) -> DialogResult<()> {
        if let Ok(Some(mut call)) = self.registry.get_session(&session_id).await {
            let old_state = call.state.clone();
            call.state = new_state.clone();
            
            self.registry.register_session(session_id.clone(), call).await
                .map_err(|e| DialogError::Coordination {
                    message: format!("Failed to update session state: {}", e),
                })?;
            
            // Send state changed event
            self.send_session_event(SessionEvent::StateChanged {
                session_id,
                old_state,
                new_state,
            }).await?;
        }
        
        Ok(())
    }
    
    /// Send a session event
    async fn send_session_event(&self, event: SessionEvent) -> DialogResult<()> {
        self.session_events_tx
            .send(event)
            .await
            .map_err(|e| DialogError::Coordination {
                message: format!("Failed to send session event: {}", e),
            })?;
            
        Ok(())
    }
    
    /// Create a mock manager for circular dependency resolution
    /// TODO: Fix this properly by restructuring dependencies
    fn create_mock_manager(&self) -> crate::manager::SessionManager {
        // This is a temporary hack to resolve circular dependency
        // In the real implementation, this would be handled differently
        todo!("Fix circular dependency between coordinator and manager")
    }
}

impl Clone for SessionDialogCoordinator {
    fn clone(&self) -> Self {
        Self {
            dialog_api: Arc::clone(&self.dialog_api),
            registry: Arc::clone(&self.registry),
            handler: self.handler.clone(),
            session_events_tx: self.session_events_tx.clone(),
            dialog_to_session: Arc::clone(&self.dialog_to_session),
        }
    }
}

impl std::fmt::Debug for SessionDialogCoordinator {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionDialogCoordinator")
            .field("mapped_dialogs", &self.dialog_to_session.len())
            .field("has_handler", &self.handler.is_some())
            .finish()
    }
} 