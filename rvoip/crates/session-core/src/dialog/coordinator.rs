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
use rvoip_sip_core::types::headers::HeaderAccess;
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
            
            SessionCoordinationEvent::CallCancelled { dialog_id, reason } => {
                // Handle CANCEL - this is for early dialog termination
                self.handle_call_terminated(dialog_id, reason).await?;
            }
            
            SessionCoordinationEvent::RegistrationRequest { transaction_id, from_uri, contact_uri, expires } => {
                self.handle_registration_request(transaction_id, from_uri.to_string(), contact_uri.to_string(), expires).await?;
            }
            
            SessionCoordinationEvent::ReInvite { dialog_id, transaction_id, request } => {
                self.handle_reinvite_request(dialog_id, transaction_id, request).await?;
            }
            
            SessionCoordinationEvent::AckSent { dialog_id, transaction_id, negotiated_sdp } => {
                self.handle_ack_sent(dialog_id, transaction_id, negotiated_sdp).await?;
            }
            
            SessionCoordinationEvent::AckReceived { dialog_id, transaction_id, negotiated_sdp } => {
                self.handle_ack_received(dialog_id, transaction_id, negotiated_sdp).await?;
            }
            
            SessionCoordinationEvent::CallProgress { dialog_id, status_code, reason_phrase } => {
                tracing::debug!("Call progress for dialog {}: {} {}", dialog_id, status_code, reason_phrase);
                // Progress events like 100 Trying don't change session state
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
        tracing::info!("handle_incoming_call called for dialog {}", dialog_id);
        
        // Extract From and To headers - simplified for now
        let from_uri = format!("sip:from@{}", source.ip());
        let to_uri = "sip:to@local".to_string();
        
        tracing::info!("Incoming call from dialog {}: {} -> {}", 
            dialog_id, from_uri, to_uri
        );
        
        // Create a new session for the incoming call
        let session_id = SessionId::new();
        self.dialog_to_session.insert(dialog_id.clone(), session_id.clone());
        
        tracing::info!("Created session {} for incoming call dialog {}", session_id, dialog_id);
        
        let call_session = CallSession {
            id: session_id.clone(),
            from: from_uri.clone(),
            to: to_uri.clone(),
            state: CallState::Ringing,
            started_at: Some(std::time::Instant::now()),
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
            tracing::info!("Calling handler.on_incoming_call for session {}", session_id);
            
            let incoming_call = IncomingCall {
                id: session_id.clone(),
                from: from_uri,
                to: to_uri,
                sdp: Some(String::from_utf8_lossy(request.body()).to_string()).filter(|s| !s.is_empty()),
                headers: self.extract_sip_headers(&request),
                received_at: std::time::Instant::now(),
            };
            
            let decision = handler.on_incoming_call(incoming_call).await;
            tracing::info!("Handler decision for session {}: {:?}", session_id, decision);
            self.process_call_decision(session_id, dialog_id, decision).await?;
        } else {
            tracing::warn!("No handler configured for incoming call");
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
            CallDecision::Accept(sdp_answer) => {
                // Get the call handle for this dialog and answer it
                if let Ok(call_handle) = self.dialog_api.get_call_handle(&dialog_id).await {
                    if let Err(e) = call_handle.answer(sdp_answer).await {
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
                
                // Implement call forwarding via dialog-core
                if let Ok(call_handle) = self.dialog_api.get_call_handle(&dialog_id).await {
                    // Send 302 Moved Temporarily response (Contact header would be added by dialog-core)
                    if let Err(e) = call_handle.reject(
                        rvoip_sip_core::StatusCode::MovedTemporarily,
                        Some(format!("Forwarded to {}", target))
                    ).await {
                        tracing::error!("Failed to forward incoming call for session {}: {}", session_id, e);
                        
                        // Fallback to simple rejection if forwarding fails
                        if let Err(e2) = call_handle.reject(
                            rvoip_sip_core::StatusCode::BusyHere,
                            Some("Forward failed".to_string())
                        ).await {
                            tracing::error!("Failed to reject call after forward failure: {}", e2);
                        }
                    } else {
                        tracing::info!("Successfully forwarded call for session {} to {}", session_id, target);
                    }
                } else {
                    tracing::error!("Failed to get call handle for dialog {} to forward call", dialog_id);
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
        let tx_id_str = transaction_id.to_string();
        tracing::info!("🔍 RESPONSE HANDLER: status={}, tx_id='{}', dialog={}", 
                      response.status_code(), tx_id_str, dialog_id);
        println!("🎯 SESSION COORDINATION: Received response {} for dialog {}", response.status_code(), dialog_id);
        
        // Handle BYE responses first - check transaction type before status code
        if tx_id_str.contains("BYE") && tx_id_str.contains("client") {
            tracing::info!("📞 BYE RESPONSE: Processing response to BYE request");
            
            match response.status_code() {
                200 => {
                    // Successful BYE - generate SessionTerminated for UAC
                    self.handle_bye_success(dialog_id).await?;
                }
                code if code >= 400 => {
                    // BYE failed
                    tracing::warn!("BYE request failed with {}: {}", code, response.reason().unwrap_or("Unknown"));
                    // Still try to clean up the session
                    if let Some(session_id_ref) = self.dialog_to_session.get(&dialog_id) {
                        let session_id = session_id_ref.value().clone();
                        self.update_session_state(session_id, CallState::Failed(format!("BYE failed: {}", code))).await?;
                    }
                }
                _ => {
                    tracing::debug!("Unexpected response {} to BYE", response.status_code());
                }
            }
            
            return Ok(());
        }
        
        // Handle INVITE responses
        if response.status_code() == 200 && tx_id_str.contains("INVITE") && tx_id_str.contains("client") {
            println!("🚀 SESSION COORDINATION: This is a 200 OK to INVITE - sending automatic ACK");
            
            // Extract SDP from 200 OK response body if present
            let response_body = String::from_utf8_lossy(response.body());
            if !response_body.trim().is_empty() {
                tracing::info!("📄 SESSION COORDINATION: 200 OK contains SDP body ({} bytes)", response_body.len());
                
                // Find session and send remote SDP event
                if let Some(session_id_ref) = self.dialog_to_session.get(&dialog_id) {
                    let session_id = session_id_ref.value().clone();
                    self.send_session_event(SessionEvent::SdpEvent {
                        session_id,
                        event_type: "remote_sdp_answer".to_string(),
                        sdp: response_body.to_string(),
                    }).await.unwrap_or_else(|e| {
                        tracing::error!("Failed to send remote SDP event: {}", e);
                    });
                }
            }
            
            // Send ACK for 2xx response using the proper dialog-core API
            match self.dialog_api.send_ack_for_2xx_response(&dialog_id, &transaction_id, &response).await {
                Ok(_) => {
                    println!("✅ SESSION COORDINATION: Successfully sent ACK for 200 OK response");
                    tracing::info!("ACK sent successfully for dialog {} transaction {}", dialog_id, transaction_id);
                    
                    // RFC 3261: Trigger UAC side media creation directly since ACK was sent
                    let negotiated_sdp = if !response_body.trim().is_empty() {
                        Some(response_body.to_string())
                    } else {
                        None
                    };
                    
                    if let Err(e) = self.handle_ack_sent(dialog_id.clone(), transaction_id.clone(), negotiated_sdp).await {
                        tracing::error!("Failed to handle ACK sent: {}", e);
                    } else {
                        tracing::info!("🚀 RFC 3261: Handled ACK sent for UAC side media creation");
                    }
                }
                Err(e) => {
                    println!("❌ SESSION COORDINATION: Failed to send ACK: {}", e);
                    tracing::error!("Failed to send ACK for dialog {} transaction {}: {}", dialog_id, transaction_id, e);
                }
            }
            
            // CRITICAL FIX: Update session state to Active for outgoing calls
            if let Some(session_id_ref) = self.dialog_to_session.get(&dialog_id) {
                let session_id = session_id_ref.value().clone();
                println!("📞 SESSION COORDINATION: Updating session {} state to Active for successful outgoing call", session_id);
                
                                                    // DON'T update to Active yet - wait for media creation after ACK!
                tracing::info!("📞 200 OK received for session {} - keeping in Initiating state until media ready", session_id);
            } else {
                tracing::debug!("No session found for dialog {} - trying alternative correlation", dialog_id);
                
                // ALTERNATIVE APPROACH: Try to find session by checking all active sessions
                // Look for sessions in Initiating state and update the first one to Active
                // This handles the timing issue where responses arrive before dialog mapping
                match self.registry.list_active_sessions().await {
                    Ok(session_ids) => {
                        for session_id in session_ids {
                            if let Ok(Some(session)) = self.registry.get_session(&session_id).await {
                                if matches!(session.state, CallState::Initiating) {
                                    tracing::info!("Alternative correlation: mapping dialog {} to session {}", dialog_id, session_id);
                                    
                                    // Map this dialog to the session for future reference
                                    self.dialog_to_session.insert(dialog_id.clone(), session_id.clone());
                                    
                                                                                                    // DON'T update to Active yet - wait for media creation after ACK!
                    tracing::info!("📞 200 OK received for session {} via alternative correlation - keeping in Initiating state until media ready", session_id);
                                    break; // Found and updated one session, stop looking
                                }
                            }
                        }
                    }
                    Err(e) => {
                        tracing::warn!("Alternative lookup failed to list active sessions: {}", e);
                    }
                }
            }
        }
        
        // Handle other response codes for non-BYE, non-INVITE requests
        match response.status_code() {
            200 => {
                // 200 OK for non-INVITE, non-BYE requests (INFO, UPDATE, etc.)
                // These responses are handled correctly by the protocol stack
                tracing::debug!("✅ RFC 3261: Successfully processed 200 OK response for dialog {}", dialog_id);
            }
            
            180 => {
                // 180 Ringing
                if let Some(session_id_ref) = self.dialog_to_session.get(&dialog_id) {
                    let session_id = session_id_ref.value().clone();
                    tracing::info!("Call ringing for session {}", session_id);
                    self.update_session_state(session_id, CallState::Ringing).await?;
                }
            }
            
            183 => {
                // 183 Session Progress
                if let Some(session_id_ref) = self.dialog_to_session.get(&dialog_id) {
                    let session_id = session_id_ref.value().clone();
                    tracing::info!("Session progress for session {}", session_id);
                    // Keep in Initiating state but could add a "Progress" state if needed
                }
            }
            
            code if code >= 400 => {
                // Call failed
                if let Some(session_id_ref) = self.dialog_to_session.get(&dialog_id) {
                    let session_id = session_id_ref.value().clone();
                    let reason = format!("{} {}", code, response.reason().unwrap_or("Unknown Error"));
                    tracing::info!("Call failed for session {}: {}", session_id, reason);
                    self.update_session_state(session_id, CallState::Failed(reason)).await?;
                }
            }
            
            _ => {
                tracing::debug!("Unhandled response {} for dialog {}", response.status_code(), dialog_id);
            }
        }
        
        Ok(())
    }
    
    /// Handle call answered coordination event (200 OK sent)
    /// NOTE: This does NOT create media sessions - wait for ACK per RFC 3261
    async fn handle_call_answered(
        &self,
        dialog_id: DialogId,
        session_answer: String,
    ) -> DialogResult<()> {
        if let Some(session_id_ref) = self.dialog_to_session.get(&dialog_id) {
            let session_id = session_id_ref.value().clone();
            tracing::info!("Call answered for session {}: {} (awaiting ACK per RFC 3261)", session_id, dialog_id);
            
            // Store the remote SDP answer
            if !session_answer.trim().is_empty() {
                self.send_session_event(SessionEvent::SdpEvent {
                    session_id: session_id.clone(),
                    event_type: "remote_sdp_answer".to_string(),
                    sdp: session_answer.clone(),
                }).await.unwrap_or_else(|e| {
                    tracing::error!("Failed to send remote SDP event: {}", e);
                });
            }
            
            // DON'T update to Active yet - wait for media creation after ACK!
            tracing::info!("📞 Call answered for session {} - keeping in Initiating state until media ready", session_id);
            
            // RFC 3261: Media should only start after ACK is received, not after 200 OK
            tracing::info!("🚫 RFC 3261: NOT creating media session yet - waiting for ACK");
        }
        
        Ok(())
    }
    
    /// Handle ACK sent coordination event (UAC side - RFC compliant media start)
    async fn handle_ack_sent(
        &self,
        dialog_id: DialogId,
        _transaction_id: rvoip_dialog_core::TransactionKey,
        negotiated_sdp: Option<String>,
    ) -> DialogResult<()> {
        if let Some(session_id_ref) = self.dialog_to_session.get(&dialog_id) {
            let session_id = session_id_ref.value().clone();
            tracing::info!("✅ RFC 3261: ACK SENT for session {} - creating media session (UAC side)", session_id);
            
            // Store final negotiated SDP if provided
            if let Some(ref sdp) = negotiated_sdp {
                self.send_session_event(SessionEvent::SdpEvent {
                    session_id: session_id.clone(),
                    event_type: "final_negotiated_sdp".to_string(),
                    sdp: sdp.clone(),
                }).await.unwrap_or_else(|e| {
                    tracing::error!("Failed to send final negotiated SDP event: {}", e);
                });
            }
            
            // RFC 3261 COMPLIANT: Create media session after ACK is sent (UAC side)
            self.send_session_event(SessionEvent::MediaEvent {
                session_id: session_id.clone(),
                event: "rfc_compliant_media_creation_uac".to_string(),
            }).await.unwrap_or_else(|e| {
                tracing::error!("Failed to send RFC compliant media create event: {}", e);
            });
        } else {
            tracing::debug!("🔍 ACK SENT: No direct session mapping found for dialog {} - using alternative correlation", dialog_id);
            
            // CRITICAL FIX: Handle media creation even with alternative correlation
            // Look for the session that was established via alternative correlation
            match self.registry.list_active_sessions().await {
                Ok(session_ids) => {
                    for session_id in session_ids {
                        if let Ok(Some(session)) = self.registry.get_session(&session_id).await {
                            if matches!(session.state, CallState::Initiating | CallState::Active) {
                                tracing::info!("🔧 ACK SENT: Alternative media creation for session {} after ACK (state: {:?})", session_id, session.state);
                                
                                // Store final negotiated SDP if provided
                                if let Some(ref sdp) = negotiated_sdp {
                                    self.send_session_event(SessionEvent::SdpEvent {
                                        session_id: session_id.clone(),
                                        event_type: "final_negotiated_sdp".to_string(),
                                        sdp: sdp.clone(),
                                    }).await.unwrap_or_else(|e| {
                                        tracing::error!("Failed to send final negotiated SDP event: {}", e);
                                    });
                                }
                                
                                // RFC 3261 COMPLIANT: Create media session after ACK is sent (UAC side)
                                self.send_session_event(SessionEvent::MediaEvent {
                                    session_id: session_id.clone(),
                                    event: "rfc_compliant_media_creation_uac".to_string(),
                                }).await.unwrap_or_else(|e| {
                                    tracing::error!("Failed to send RFC compliant media create event: {}", e);
                                });
                                
                                // Also map this dialog for future reference
                                self.dialog_to_session.insert(dialog_id.clone(), session_id.clone());
                                
                                break; // Only create media for one session (first Active one found)
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to find session for ACK sent alternative correlation: {}", e);
                }
            }
        }
        
        Ok(())
    }
    
    /// Handle ACK received coordination event (UAS side - RFC compliant media start)
    async fn handle_ack_received(
        &self,
        dialog_id: DialogId,
        _transaction_id: rvoip_dialog_core::TransactionKey,
        negotiated_sdp: Option<String>,
    ) -> DialogResult<()> {
        if let Some(session_id_ref) = self.dialog_to_session.get(&dialog_id) {
            let session_id = session_id_ref.value().clone();
            tracing::info!("✅ RFC 3261: ACK RECEIVED for session {} - creating media session (UAS side)", session_id);
            
            // Store final negotiated SDP if provided
            if let Some(ref sdp) = negotiated_sdp {
                self.send_session_event(SessionEvent::SdpEvent {
                    session_id: session_id.clone(),
                    event_type: "final_negotiated_sdp".to_string(),
                    sdp: sdp.clone(),
                }).await.unwrap_or_else(|e| {
                    tracing::error!("Failed to send final negotiated SDP event: {}", e);
                });
            }
            
            // RFC 3261 COMPLIANT: Create media session after ACK is received (UAS side)
            self.send_session_event(SessionEvent::MediaEvent {
                session_id: session_id.clone(),
                event: "rfc_compliant_media_creation_uas".to_string(),
            }).await.unwrap_or_else(|e| {
                tracing::error!("Failed to send RFC compliant media create event: {}", e);
            });
        } else {
            tracing::debug!("🔍 ACK RECEIVED: No direct session mapping found for dialog {} - using alternative correlation", dialog_id);
            
            // CRITICAL FIX: Handle media creation even with alternative correlation  
            // Look for the session that was established via alternative correlation
            match self.registry.list_active_sessions().await {
                Ok(session_ids) => {
                                         for session_id in session_ids {
                         if let Ok(Some(session)) = self.registry.get_session(&session_id).await {
                             if matches!(session.state, CallState::Initiating | CallState::Active) {
                                 tracing::info!("🔧 ACK RECEIVED: Alternative media creation for session {} after ACK (state: {:?})", session_id, session.state);
                                
                                // Store final negotiated SDP if provided
                                if let Some(ref sdp) = negotiated_sdp {
                                    self.send_session_event(SessionEvent::SdpEvent {
                                        session_id: session_id.clone(),
                                        event_type: "final_negotiated_sdp".to_string(),
                                        sdp: sdp.clone(),
                                    }).await.unwrap_or_else(|e| {
                                        tracing::error!("Failed to send final negotiated SDP event: {}", e);
                                    });
                                }
                                
                                // RFC 3261 COMPLIANT: Create media session after ACK is received (UAS side)
                                self.send_session_event(SessionEvent::MediaEvent {
                                    session_id: session_id.clone(),
                                    event: "rfc_compliant_media_creation_uas".to_string(),
                                }).await.unwrap_or_else(|e| {
                                    tracing::error!("Failed to send RFC compliant media create event: {}", e);
                                });
                                
                                // Also map this dialog for future reference
                                self.dialog_to_session.insert(dialog_id.clone(), session_id.clone());
                                
                                break; // Only create media for one session (first Active one found)
                            }
                        }
                    }
                }
                Err(e) => {
                    tracing::error!("Failed to find session for ACK received alternative correlation: {}", e);
                }
            }
        }
        
        Ok(())
    }
    
    /// Handle call terminated coordination event
    async fn handle_call_terminated(
        &self,
        dialog_id: DialogId,
        reason: String,
    ) -> DialogResult<()> {
        tracing::info!("handle_call_terminated called for dialog {}: {}", dialog_id, reason);
        
        if let Some((_, session_id)) = self.dialog_to_session.remove(&dialog_id) {
            tracing::info!("Call terminated for session {}: {} - {}", session_id, dialog_id, reason);
            
            // Send session terminated event
            self.send_session_event(SessionEvent::SessionTerminated {
                session_id: session_id.clone(),
                reason: reason.clone(),
            }).await?;
            
            // Don't unregister here - let the main coordinator do it after handler notification
            // self.registry.unregister_session(&session_id).await
            //     .map_err(|e| DialogError::Coordination {
            //         message: format!("Failed to unregister session: {}", e),
            //     })?;
        } else {
            tracing::warn!("No session found for terminated dialog {}", dialog_id);
        }
        
        Ok(())
    }
    
    /// Handle successful BYE response (200 OK) for UAC
    async fn handle_bye_success(&self, dialog_id: DialogId) -> DialogResult<()> {
        tracing::info!("📞 BYE SUCCESS: Processing successful BYE for dialog {}", dialog_id);
        println!("📞 BYE SUCCESS: Processing successful BYE for dialog {}", dialog_id);
        
        // Look up session - get it before removing the mapping!
        let session_info = self.dialog_to_session.get(&dialog_id)
            .map(|entry| entry.value().clone());
            
        if let Some(session_id) = session_info {
            tracing::info!("✅ BYE SUCCESS: Found session {} for dialog {}", session_id, dialog_id);
            
            // Send SessionTerminated event for the UAC
            self.send_session_event(SessionEvent::SessionTerminated {
                session_id: session_id.clone(),
                reason: "Call terminated by local BYE".to_string(),
            }).await.unwrap_or_else(|e| {
                tracing::error!("Failed to send SessionTerminated event: {}", e);
            });
            
            tracing::info!("✅ BYE SUCCESS: Generated SessionTerminated event for UAC session {}", session_id);
            println!("✅ BYE SUCCESS: Generated SessionTerminated event for UAC session {}", session_id);
            
            // NOW remove the dialog mapping after sending the event
            self.dialog_to_session.remove(&dialog_id);
            tracing::debug!("Removed dialog mapping for {}", dialog_id);
        } else {
            tracing::warn!("❌ BYE SUCCESS: No session found for dialog {} - mapping may have been removed prematurely", dialog_id);
            println!("❌ BYE SUCCESS: No session found for dialog {}", dialog_id);
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
    
    /// Handle ReInvite requests (including INFO for DTMF and UPDATE for hold)
    async fn handle_reinvite_request(
        &self,
        dialog_id: DialogId,
        transaction_id: rvoip_dialog_core::TransactionKey,
        request: rvoip_sip_core::Request,
    ) -> DialogResult<()> {
        let method = request.method();
        tracing::info!("ReInvite request {} for dialog {}", method, dialog_id);
        
        match method {
            rvoip_sip_core::Method::Info => {
                self.handle_info_request(dialog_id, transaction_id, request).await?;
            }
            
            rvoip_sip_core::Method::Update => {
                self.handle_update_request(dialog_id, transaction_id, request).await?;
            }
            
            rvoip_sip_core::Method::Invite => {
                self.handle_invite_reinvite(dialog_id, transaction_id, request).await?;
            }
            
            _ => {
                tracing::warn!("Unhandled ReInvite method {} for dialog {}", method, dialog_id);
                // Send 501 Not Implemented response
                if let Err(e) = self.send_response(&transaction_id, 501, "Not Implemented").await {
                    tracing::error!("Failed to send 501 response: {}", e);
                }
            }
        }
        
        Ok(())
    }
    
    /// Handle INFO request (typically DTMF)
    async fn handle_info_request(
        &self,
        dialog_id: DialogId,
        transaction_id: rvoip_dialog_core::TransactionKey,
        request: rvoip_sip_core::Request,
    ) -> DialogResult<()> {
        tracing::info!("Handling INFO request for dialog {}", dialog_id);
        
        // Extract DTMF from request body
        let body = String::from_utf8_lossy(request.body());
        if body.starts_with("DTMF:") {
            let dtmf_digits = body.strip_prefix("DTMF:").unwrap_or("").trim();
            tracing::info!("Received DTMF: '{}' for dialog {}", dtmf_digits, dialog_id);
            
            // Find the session and notify handler if available
            if let Some(session_id_ref) = self.dialog_to_session.get(&dialog_id) {
                let session_id = session_id_ref.value().clone();
                
                // Send DTMF received event
                self.send_session_event(SessionEvent::DtmfReceived {
                    session_id,
                    digits: dtmf_digits.to_string(),
                }).await?;
            }
        }
        
        // Send 200 OK response
        if let Err(e) = self.send_response(&transaction_id, 200, "OK").await {
            tracing::error!("Failed to send 200 OK response to INFO: {}", e);
        }
        
        Ok(())
    }
    
    /// Handle UPDATE request (typically for hold/resume)
    async fn handle_update_request(
        &self,
        dialog_id: DialogId,
        transaction_id: rvoip_dialog_core::TransactionKey,
        request: rvoip_sip_core::Request,
    ) -> DialogResult<()> {
        tracing::info!("Handling UPDATE request for dialog {}", dialog_id);
        
        // Check if this is a hold request by examining SDP
        let body = String::from_utf8_lossy(request.body());
        let is_hold = body.contains("hold") || body.contains("sendonly") || body.contains("inactive");
        
        if let Some(session_id_ref) = self.dialog_to_session.get(&dialog_id) {
            let session_id = session_id_ref.value().clone();
            
            if is_hold {
                tracing::info!("Hold request detected for session {}", session_id);
                self.update_session_state(session_id.clone(), CallState::OnHold).await?;
                
                // Send hold event
                self.send_session_event(SessionEvent::SessionHeld {
                    session_id,
                }).await?;
            } else {
                tracing::info!("Resume request detected for session {}", session_id);
                self.update_session_state(session_id.clone(), CallState::Active).await?;
                
                // Send resume event
                self.send_session_event(SessionEvent::SessionResumed {
                    session_id,
                }).await?;
            }
        }
        
        // Send 200 OK response without SDP body (UPDATE doesn't require SDP answer)
        if let Err(e) = self.send_response(&transaction_id, 200, "OK").await {
            tracing::error!("Failed to send 200 OK response to UPDATE: {}", e);
        }
        
        Ok(())
    }
    
    /// Handle re-INVITE request (media changes)
    async fn handle_invite_reinvite(
        &self,
        dialog_id: DialogId,
        transaction_id: rvoip_dialog_core::TransactionKey,
        request: rvoip_sip_core::Request,
    ) -> DialogResult<()> {
        tracing::info!("Handling re-INVITE request for dialog {}", dialog_id);
        
        // Extract the SDP offer from the re-INVITE
        let offered_sdp = String::from_utf8_lossy(request.body());
        
        if let Some(session_id_ref) = self.dialog_to_session.get(&dialog_id) {
            let session_id = session_id_ref.value().clone();
            tracing::info!("Processing re-INVITE for session {}", session_id);
            
            // Send media update event to be handled by SessionManager
            // The SessionManager will coordinate with MediaCoordinator for SDP answer
            self.send_session_event(SessionEvent::MediaUpdate {
                session_id: session_id.clone(),
                offered_sdp: if offered_sdp.trim().is_empty() { 
                    None 
                } else { 
                    Some(offered_sdp.to_string()) 
                },
            }).await?;
            
            // For now, send 200 OK without SDP body
            // In a complete implementation, the SessionManager would generate 
            // the SDP answer via MediaCoordinator and send it back
            if let Err(e) = self.send_response(&transaction_id, 200, "OK").await {
                tracing::error!("Failed to send 200 OK response to re-INVITE: {}", e);
            } else {
                tracing::info!("Successfully responded to re-INVITE for session {}", session_id);
            }
        } else {
            tracing::warn!("No session found for dialog {} in re-INVITE", dialog_id);
            
            // Send 481 Call/Transaction Does Not Exist
            if let Err(e) = self.send_response(&transaction_id, 481, "Call/Transaction Does Not Exist").await {
                tracing::error!("Failed to send 481 response to re-INVITE: {}", e);
            }
        }
        
        Ok(())
    }
    
    /// Send a simple response
    async fn send_response(
        &self,
        transaction_id: &rvoip_dialog_core::TransactionKey,
        status_code: u16,
        reason_phrase: &str,
    ) -> Result<(), String> {
        // Use dialog-core API to send status response
        let status = match status_code {
            200 => rvoip_sip_core::StatusCode::Ok,
            481 => rvoip_sip_core::StatusCode::CallOrTransactionDoesNotExist,
            501 => rvoip_sip_core::StatusCode::NotImplemented,
            _ => rvoip_sip_core::StatusCode::Ok, // Default to OK
        };
        
        self.dialog_api
            .send_status_response(transaction_id, status, Some(reason_phrase.to_string()))
            .await
            .map_err(|e| format!("Failed to send response: {}", e))
    }
    
    /// Send a response with body content  
    async fn send_response_with_body(
        &self,
        transaction_id: &rvoip_dialog_core::TransactionKey,
        status_code: u16,
        reason_phrase: &str,
        body: &str,
        content_type: &str,
    ) -> Result<(), String> {
        // Build proper response with body using dialog-core API
        let status = match status_code {
            200 => rvoip_sip_core::StatusCode::Ok,
            _ => rvoip_sip_core::StatusCode::Ok,
        };
        
        // Build response with proper headers and body
        let response = self.dialog_api
            .build_response(transaction_id, status, Some(body.to_string()))
            .await
            .map_err(|e| format!("Failed to build response: {}", e))?;
            
        self.dialog_api
            .send_response(transaction_id, response)
            .await
            .map_err(|e| format!("Failed to send response with body: {}", e))
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
    
    /// Extract relevant SIP headers from request
    fn extract_sip_headers(&self, _request: &rvoip_sip_core::Request) -> std::collections::HashMap<String, String> {
        // For now, return empty headers map
        // TODO: Implement proper header extraction when needed
        // The complex header API makes this non-trivial, so we'll defer this
        std::collections::HashMap::new()
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