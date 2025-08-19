//! Session Dialog Coordinator (parallel to SessionMediaCoordinator)
//!
//! Manages the coordination between session-core and dialog-core,
//! handling event bridging and lifecycle management.

use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
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
use crate::session::Session;
use crate::manager::events::SessionEvent;
use crate::coordinator::registry::InternalSessionRegistry;
use crate::coordinator::transfer::TransferHandler;
use crate::dialog::{DialogError, DialogResult};
use dashmap::DashMap;
use tracing;

/// Session dialog coordinator for automatic dialog lifecycle management
/// (parallel to SessionMediaCoordinator)
pub struct SessionDialogCoordinator {
    dialog_api: Arc<UnifiedDialogApi>,
    registry: Arc<InternalSessionRegistry>,
    handler: Option<Arc<dyn CallHandler>>,
    // Removed mpsc sender - now using unified event processor only
    dialog_to_session: Arc<dashmap::DashMap<DialogId, SessionId>>,
    session_to_dialog: Arc<dashmap::DashMap<SessionId, DialogId>>,
    // Store incoming SDP offers for negotiation
    incoming_sdp_offers: Arc<DashMap<SessionId, String>>,
    // CRITICAL FIX: Track Call-ID to session mapping for dialog transitions
    callid_to_session: Arc<dashmap::DashMap<String, SessionId>>,
    // WORKAROUND: Track From tag to session for outgoing calls since dialog IDs change
    fromtag_to_session: Arc<dashmap::DashMap<String, SessionId>>,
    // Track From URI for faster UAC-side correlation
    fromuri_to_session: Arc<dashmap::DashMap<String, SessionId>>,
    // Transfer handler
    pub transfer_handler: Arc<TransferHandler>,
    // Event processor (passed in from coordinator)
    event_processor: Arc<crate::manager::events::SessionEventProcessor>,
}

impl SessionDialogCoordinator {
    /// Create a new session dialog coordinator
    pub fn new(
        dialog_api: Arc<UnifiedDialogApi>,
        registry: Arc<InternalSessionRegistry>,
        handler: Option<Arc<dyn CallHandler>>,
        event_processor: Arc<crate::manager::events::SessionEventProcessor>,
        dialog_to_session: Arc<dashmap::DashMap<DialogId, SessionId>>,
        session_to_dialog: Arc<dashmap::DashMap<SessionId, DialogId>>,
        incoming_sdp_offers: Arc<DashMap<SessionId, String>>,
    ) -> Self {
        // Create transfer handler with event processor
        let transfer_handler = Arc::new(TransferHandler::new(
            dialog_api.clone(),
            registry.clone(),
            dialog_to_session.clone(),
            session_to_dialog.clone(),
            event_processor.clone(),
        ));
        
        Self {
            dialog_api,
            registry,
            handler,
            // Removed mpsc sender field
            dialog_to_session,
            session_to_dialog,
            incoming_sdp_offers,
            callid_to_session: Arc::new(dashmap::DashMap::new()),
            fromtag_to_session: Arc::new(dashmap::DashMap::new()),
            fromuri_to_session: Arc::new(dashmap::DashMap::new()),
            transfer_handler,
            event_processor,
        }
    }
    
    /// Get access to the dialog API
    pub fn dialog_api(&self) -> &Arc<UnifiedDialogApi> {
        &self.dialog_api
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
            
            SessionCoordinationEvent::CallTerminating { dialog_id, reason } => {
                self.handle_call_terminating(dialog_id, reason).await?;
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
            
            SessionCoordinationEvent::CleanupConfirmation { dialog_id, layer } => {
                self.handle_cleanup_confirmation(dialog_id, layer).await?;
            }
            
            SessionCoordinationEvent::TransferRequest { 
                dialog_id, 
                transaction_id, 
                refer_to, 
                referred_by, 
                replaces 
            } => {
                println!("🔀 TRANSFER: Received TransferRequest event for dialog {}", dialog_id);
                tracing::info!("Received TransferRequest event for dialog {}", dialog_id);
                println!("🔀 TRANSFER: Calling transfer_handler.handle_refer_request");
                match self.transfer_handler.handle_refer_request(
                    dialog_id,
                    transaction_id,
                    refer_to,
                    referred_by,
                    replaces,
                ).await {
                    Ok(()) => {
                        println!("✅ TRANSFER: Successfully handled transfer request");
                    },
                    Err(e) => {
                        println!("❌ TRANSFER: Failed to handle transfer request: {}", e);
                        return Err(DialogError::Coordination {
                            message: format!("Failed to handle transfer request: {}", e),
                        });
                    }
                }
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
        
        // Extract From and To headers properly from the SIP request
        let from_uri = request.from()
            .map(|h| h.address().uri.to_string())
            .unwrap_or_else(|| format!("sip:unknown@{}", source.ip()));
            
        let to_uri = request.to()
            .map(|h| h.address().uri.to_string())
            .unwrap_or_else(|| "sip:unknown@local".to_string());
        
        tracing::info!("Incoming call from dialog {}: {} -> {}", 
            dialog_id, from_uri, to_uri
        );
        
        // IMPORTANT: Do NOT create a session here!
        // The main coordinator (coordinator/event_handler.rs) will create the session
        // when it processes the IncomingCall event that we forward below.
        // We just create a session ID to track the dialog-to-session mapping.
        let session_id = SessionId::new();
        
        // Track dialog to session mapping
        self.dialog_to_session.insert(dialog_id.clone(), session_id.clone());
        
        // PHASE 17.4: Also track session to dialog mapping for BYE lookups
        self.session_to_dialog.insert(session_id.clone(), dialog_id.clone());
        
        // CRITICAL FIX: Track From tag and Call-ID for incoming calls (UAS side)
        // When we receive an INVITE as UAS, the From tag is the remote tag
        if let Some(from_header) = request.from() {
            if let Some(from_tag) = from_header.tag() {
                let tag_str = from_tag.to_string();
                self.fromtag_to_session.insert(tag_str.clone(), session_id.clone());
                tracing::info!("📞 UAS: Stored From tag {} for incoming session {}", tag_str, session_id);
            }
        }
        
        // Also track Call-ID for fallback correlation
        if let Some(call_id) = request.call_id() {
            let call_id_str = call_id.to_string();
            self.callid_to_session.insert(call_id_str.clone(), session_id.clone());
            tracing::info!("📞 UAS: Stored Call-ID {} for incoming session {}", call_id_str, session_id);
        }
        
        tracing::info!("Dialog coordinator mapped dialog {} to session {} (will be created by main coordinator)", dialog_id, session_id);
        
        // DO NOT create or register session here!
        // The main coordinator will do that when processing the IncomingCall event.
        
        // Forward the IncomingCall event to the main coordinator
        // The main coordinator will:
        // 1. Create the session
        // 2. Register it
        // 3. Send SessionCreated event
        // 4. Call the handler
        self.send_session_event(SessionEvent::IncomingCall {
            session_id: session_id.clone(),
            dialog_id: dialog_id.clone(),
            from: from_uri,
            to: to_uri,
            sdp: Some(String::from_utf8_lossy(request.body()).to_string()).filter(|s| !s.is_empty()),
            headers: self.extract_sip_headers(&request),
        }).await?;
        
        tracing::info!("Forwarded IncomingCall event to main coordinator for session {}", session_id);
        
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
                // Get the call handle for this dialog
                if let Ok(call_handle) = self.dialog_api.get_call_handle(&dialog_id).await {
                    // If no SDP answer provided, generate one
                    let final_sdp_answer = if sdp_answer.is_none() {
                        // Get the stored incoming SDP offer
                        if let Some(their_offer) = self.incoming_sdp_offers.get(&session_id) {
                            tracing::info!("Generating SDP answer for session {} as UAS", session_id);
                            
                            // Use the parent coordinator to generate SDP answer
                            // The coordinator has the media manager and negotiator
                            match self.send_session_event(SessionEvent::SdpNegotiationRequested {
                                session_id: session_id.clone(),
                                role: "uas".to_string(),
                                local_sdp: None,
                                remote_sdp: Some(their_offer.value().clone()),
                            }).await {
                                Ok(_) => {
                                    // The negotiation happens asynchronously
                                    // For now we have to accept without SDP and let the coordinator handle it
                                    tracing::warn!("SDP negotiation requested, accepting without SDP for now");
                                    None
                                }
                                Err(e) => {
                                    tracing::error!("Failed to request SDP negotiation: {}", e);
                                    None
                                }
                            }
                        } else {
                            tracing::warn!("No incoming SDP offer stored for session {}, accepting without SDP", session_id);
                            None
                        }
                    } else {
                        sdp_answer
                    };
                    
                    // Clean up the stored SDP offer
                    self.incoming_sdp_offers.remove(&session_id);
                    
                    // Now answer with the SDP (or None if generation failed)
                    if let Err(e) = call_handle.answer(final_sdp_answer.clone()).await {
                        tracing::error!("Failed to answer incoming call for session {}: {}", session_id, e);
                    } else {
                        tracing::info!("Answered incoming call for session {} with SDP: {}", 
                                     session_id, final_sdp_answer.is_some());
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
            tracing::info!("📞 BYE-RESPONSE: Processing response to BYE request (tx: {}, dialog: {})", tx_id_str, dialog_id);
            
            match response.status_code() {
                200 => {
                    // PHASE 0.24: Enhanced BYE success logging
                    tracing::info!("✅ BYE-200OK: Received 200 OK for BYE request (tx: {}, dialog: {})", tx_id_str, dialog_id);
                    
                    // Log timing information if available
                    if let Some(session_id_ref) = self.dialog_to_session.get(&dialog_id) {
                        let session_id = session_id_ref.value().clone();
                        tracing::info!("🎯 BYE-COMPLETE: Session {} successfully terminated with 200 OK", session_id);
                    }
                    
                    // Successful BYE - generate SessionTerminated for UAC
                    self.handle_bye_success(dialog_id).await?;
                }
                code if code >= 400 => {
                    // PHASE 0.24: Enhanced BYE failure logging
                    let reason = response.reason().unwrap_or("Unknown");
                    tracing::warn!("❌ BYE-ERROR: BYE request failed with {} {}: {} (tx: {}, dialog: {})", 
                                 code, reason, response.reason().unwrap_or("No reason"), tx_id_str, dialog_id);
                    
                    if let Some(session_id_ref) = self.dialog_to_session.get(&dialog_id) {
                        let session_id = session_id_ref.value().clone();
                        tracing::warn!("💥 BYE-FAILED: Session {} termination failed: {} {}", session_id, code, reason);
                    }
                    
                    // Still try to clean up the session even if BYE failed
                    if let Some(session_id_ref) = self.dialog_to_session.get(&dialog_id) {
                        let session_id = session_id_ref.value().clone();
                        self.update_session_state(session_id, CallState::Failed(format!("BYE failed: {}", code))).await?;
                    }
                }
                code if code >= 100 && code < 200 => {
                    // PHASE 0.24: Track provisional responses to BYE
                    tracing::debug!("📞 BYE-PROVISIONAL: Received {} {} for BYE (tx: {}, dialog: {})", 
                                  code, response.reason().unwrap_or(""), tx_id_str, dialog_id);
                }
                _ => {
                    tracing::debug!("🤔 BYE-UNEXPECTED: Unexpected response {} to BYE (tx: {}, dialog: {})", 
                                  response.status_code(), tx_id_str, dialog_id);
                }
            }
            
            return Ok(());
        }
        
        // CRITICAL: Track From tag for responses to help find sessions when dialog IDs change
        // This is a workaround for dialog-core creating new dialog IDs on state transitions
        println!("🔍 Checking dialog {} for From tag tracking", dialog_id);
        
        // Try to store From tag mapping if we know this dialog
        if let Some(session_id_ref) = self.dialog_to_session.get(&dialog_id) {
            let session_id = session_id_ref.value().clone();
            
            // Extract From tag if present
            if let Some(from_header) = response.from() {
                if let Some(tag) = from_header.tag() {
                    let from_tag = tag.to_string();
                    if !self.fromtag_to_session.contains_key(&from_tag) {
                        self.fromtag_to_session.insert(from_tag.clone(), session_id.clone());
                        println!("📞 Stored From tag {} for session {}", from_tag, session_id);
                    }
                }
            }
            
            // Also store Call-ID for completeness
            if let Some(call_id_header) = response.call_id() {
                let call_id_str = call_id_header.0.clone();
                if !self.callid_to_session.contains_key(&call_id_str) {
                    self.callid_to_session.insert(call_id_str.clone(), session_id.clone());
                    println!("📞 Stored Call-ID {} for session {}", call_id_str, session_id);
                }
            }
        } else {
            println!("❌ No session found for dialog {} to store From tag", dialog_id);
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
            println!("🔍 DIALOG COORDINATOR: Looking for session mapped to dialog {}", dialog_id);
            
            // Try to find session first by dialog ID
            let session_id = if let Some(session_id_ref) = self.dialog_to_session.get(&dialog_id) {
                let sid = session_id_ref.value().clone();
                println!("✅ DIALOG COORDINATOR: Found session {} for dialog {}", sid, dialog_id);
                Some(sid)
            } else {
                // Dialog ID not found - try From tag lookup
                // This happens when dialog transitions from Early to Confirmed  
                if let Some(from_header) = response.from() {
                    if let Some(tag) = from_header.tag() {
                        let from_tag = tag.to_string();
                        println!("🔍 DIALOG COORDINATOR: Dialog {} not found, trying From tag {}", dialog_id, from_tag);
                        
                        if let Some(session_id_ref) = self.fromtag_to_session.get(&from_tag) {
                            let sid = session_id_ref.value().clone();
                            println!("✅ DIALOG COORDINATOR: Found session {} via From tag {}", sid, from_tag);
                            
                            // Update mappings with new dialog ID
                            self.dialog_to_session.insert(dialog_id.clone(), sid.clone());
                            self.session_to_dialog.insert(sid.clone(), dialog_id.clone());
                            
                            // Also store From tag for this new dialog  
                            self.fromtag_to_session.insert(from_tag, sid.clone());
                            
                            Some(sid)
                        } else {
                            // Try Call-ID as final fallback
                            if let Some(call_id_header) = response.call_id() {
                                let call_id_str = call_id_header.0.clone();
                                println!("🔍 DIALOG COORDINATOR: From tag {} not found, trying Call-ID {}", from_tag, call_id_str);
                                
                                if let Some(session_id_ref) = self.callid_to_session.get(&call_id_str) {
                                    let sid = session_id_ref.value().clone();
                                    println!("✅ DIALOG COORDINATOR: Found session {} via Call-ID {}", sid, call_id_str);
                                    
                                    // Update all mappings
                                    self.dialog_to_session.insert(dialog_id.clone(), sid.clone());
                                    self.session_to_dialog.insert(sid.clone(), dialog_id.clone());
                                    self.fromtag_to_session.insert(from_tag, sid.clone());
                                    
                                    Some(sid)
                                } else {
                                    println!("❌ DIALOG COORDINATOR: No session found for From tag {} or Call-ID {}", from_tag, call_id_str);
                                    None
                                }
                            } else {
                                println!("❌ DIALOG COORDINATOR: No session found for From tag {}", from_tag);
                                None
                            }
                        }
                    } else {
                        println!("❌ DIALOG COORDINATOR: No From tag in response");
                        None
                    }
                } else {
                    println!("❌ DIALOG COORDINATOR: No From header in response");
                    None
                }
            };
            
            if let Some(session_id) = session_id {
                println!("📞 SESSION COORDINATION: Processing 200 OK for session {} (UAC side)", session_id);
                
                // WORKAROUND: Since dialog-core doesn't generate AckSent events,
                // we'll trigger media creation right after sending the ACK
                // The handle_ack_sent call above should have triggered this, but
                // if session mapping failed, we need to do it here
                tracing::info!("📞 200 OK received for session {} - triggering UAC media creation", session_id);
                
                // Send MediaEvent for UAC side
                self.send_session_event(SessionEvent::MediaEvent {
                    session_id: session_id.clone(),
                    event: "rfc_compliant_media_creation_uac".to_string(),
                }).await.unwrap_or_else(|e| {
                    tracing::error!("Failed to send UAC media creation event: {}", e);
                });
            } else {
                println!("❌ DIALOG COORDINATOR: Could not find session for dialog {}", dialog_id);
                tracing::warn!("No session found for dialog {} - attempting fallback correlation", dialog_id);
                
                // FINAL FALLBACK: For 200 OK to INVITE on UAC side, use indexed From URI lookup
                // This is a workaround for dialog-core creating new dialog IDs
                if let Some(from_header) = response.from() {
                    let from_uri = from_header.address().uri.to_string();
                    
                    // Use indexed lookup instead of iterating through all sessions
                    if let Some(session_ref) = self.fromuri_to_session.get(&from_uri) {
                        let sid = session_ref.value().clone();
                        
                        // Update mappings for future use
                        self.dialog_to_session.insert(dialog_id.clone(), sid.clone());
                        self.session_to_dialog.insert(sid.clone(), dialog_id.clone());
                        
                        // Send MediaEvent for UAC side
                        self.send_session_event(SessionEvent::MediaEvent {
                            session_id: sid.clone(),
                            event: "rfc_compliant_media_creation_uac".to_string(),
                        }).await.unwrap_or_else(|e| {
                            tracing::error!("Failed to send UAC media creation event: {}", e);
                        });
                    } else {
                        tracing::warn!("No session found for From URI {} in fallback", from_uri);
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
            
            // Store our local SDP answer (as UAS, we generated this answer)
            if !session_answer.trim().is_empty() {
                self.send_session_event(SessionEvent::SdpEvent {
                    session_id: session_id.clone(),
                    event_type: "local_sdp_answer".to_string(),
                    sdp: session_answer.clone(),
                }).await.unwrap_or_else(|e| {
                    tracing::error!("Failed to send local SDP event: {}", e);
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
        transaction_id: rvoip_dialog_core::TransactionKey,
        negotiated_sdp: Option<String>,
    ) -> DialogResult<()> {
        // First try direct dialog-to-session mapping
        let session_id = if let Some(session_id_ref) = self.dialog_to_session.get(&dialog_id) {
            let sid = session_id_ref.value().clone();
            tracing::info!("✅ RFC 3261: ACK SENT for session {} via direct mapping - creating media session (UAC side)", sid);
            Some(sid)
        } else {
            // Try transaction-based correlation like we do for ACK received
            let tx_str = transaction_id.to_string();
            tracing::debug!("🔍 ACK SENT: No direct mapping for dialog {}, checking transaction {}", dialog_id, tx_str);
            
            // Extract Call-ID from transaction (format: method|call-id|from-tag|...)
            if tx_str.contains("call-") {
                let call_id_start = tx_str.find("call-").unwrap_or(0);
                let call_id_part = &tx_str[call_id_start..];
                let call_id_end = call_id_part.find('|').unwrap_or(call_id_part.len());
                let call_id = &call_id_part[..call_id_end];
                
                if let Some(session_ref) = self.callid_to_session.get(call_id) {
                    let sid = session_ref.value().clone();
                    tracing::info!("✅ ACK SENT: Found session {} via Call-ID {} correlation", sid, call_id);
                    
                    // Update dialog mapping for future use
                    self.dialog_to_session.insert(dialog_id.clone(), sid.clone());
                    self.session_to_dialog.insert(sid.clone(), dialog_id.clone());
                    
                    Some(sid)
                } else {
                    tracing::warn!("⚠️ ACK SENT: No session found for Call-ID {}", call_id);
                    None
                }
            } else {
                tracing::warn!("⚠️ ACK SENT: Could not extract Call-ID from transaction {}", tx_str);
                None
            }
        };
        
        if let Some(session_id) = session_id {
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
            tracing::warn!("⚠️ ACK SENT: No session found for dialog {} after all correlation attempts", dialog_id);
        }
        
        Ok(())
    }
    
    /// Handle ACK received coordination event (UAS side - RFC compliant media start)
    async fn handle_ack_received(
        &self,
        dialog_id: DialogId,
        transaction_id: rvoip_dialog_core::TransactionKey,
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
            
            // IMPROVED: Try transaction-based correlation
            let tx_str = transaction_id.to_string();
            tracing::info!("🔍 ACK RECEIVED: Using transaction-based correlation for dialog {}, tx: {}", dialog_id, tx_str);
            
            // Extract Call-ID from transaction (format: method|call-id|from-tag|...)
            let session_id = if tx_str.contains("call-") {
                let call_id_start = tx_str.find("call-").unwrap_or(0);
                let call_id_part = &tx_str[call_id_start..];
                let call_id_end = call_id_part.find('|').unwrap_or(call_id_part.len());
                let call_id = &call_id_part[..call_id_end];
                
                if let Some(session_ref) = self.callid_to_session.get(call_id) {
                    let sid = session_ref.value().clone();
                    tracing::info!("✅ ACK RECEIVED: Found session {} via Call-ID {} correlation", sid, call_id);
                    
                    // Update dialog mapping for future use
                    self.dialog_to_session.insert(dialog_id.clone(), sid.clone());
                    self.session_to_dialog.insert(sid.clone(), dialog_id.clone());
                    
                    Some(sid)
                } else {
                    tracing::warn!("⚠️ ACK RECEIVED: No session found for Call-ID {}", call_id);
                    None
                }
            } else {
                tracing::warn!("⚠️ ACK RECEIVED: Could not extract Call-ID from transaction {}", tx_str);
                None
            };
            
            if let Some(session_id) = session_id {
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
                tracing::warn!("⚠️ ACK RECEIVED: No session found for dialog {} after correlation attempts", dialog_id);
            }
        }
        
        Ok(())
    }
    
    /// Handle cleanup confirmation from a layer
    async fn handle_cleanup_confirmation(
        &self,
        dialog_id: DialogId,
        layer: String,
    ) -> DialogResult<()> {
        tracing::info!("Cleanup confirmation received from {} for dialog {}", layer, dialog_id);
        
        if let Some(session_id_ref) = self.dialog_to_session.get(&dialog_id) {
            let session_id = session_id_ref.value().clone();
            
            // Forward the cleanup confirmation to the session event system
            self.send_session_event(SessionEvent::CleanupConfirmation {
                session_id: session_id.clone(),
                layer,
            }).await?;
        } else {
            tracing::warn!("No session found for cleanup confirmation from dialog {}", dialog_id);
        }
        
        Ok(())
    }
    
    /// Handle call terminating coordination event (Phase 1)
    async fn handle_call_terminating(
        &self,
        dialog_id: DialogId,
        reason: String,
    ) -> DialogResult<()> {
        tracing::info!("handle_call_terminating called for dialog {} (Phase 1): {}", dialog_id, reason);
        
        if let Some(session_id_ref) = self.dialog_to_session.get(&dialog_id) {
            let session_id = session_id_ref.value().clone();
            tracing::info!("Call terminating (Phase 1) for session {}: {} - {}", session_id, dialog_id, reason);
            
            // Send session terminating event (Phase 1)
            self.send_session_event(SessionEvent::SessionTerminating {
                session_id: session_id.clone(),
                reason: reason.clone(),
            }).await?;
            
            // Keep dialog_to_session mapping for now
        } else {
            tracing::warn!("No session found for terminating dialog {}", dialog_id);
        }
        
        Ok(())
    }
    
    /// Handle call terminated coordination event (Phase 2)
    async fn handle_call_terminated(
        &self,
        dialog_id: DialogId,
        reason: String,
    ) -> DialogResult<()> {
        tracing::info!("handle_call_terminated called for dialog {} (Phase 2): {}", dialog_id, reason);
        
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
        tracing::info!("📞 BYE-SUCCESS: Processing successful BYE for dialog {}", dialog_id);
        println!("📞 BYE-SUCCESS: Processing successful BYE for dialog {}", dialog_id);
        
        // PHASE 0.24: Add detailed success metrics
        let start_time = std::time::Instant::now();
        
        // Look up session - get it before removing the mapping!
        let session_info = self.dialog_to_session.get(&dialog_id)
            .map(|entry| entry.value().clone());
            
        if let Some(session_id) = session_info {
            tracing::info!("✅ BYE-SUCCESS: Found session {} for dialog {} - termination successful", session_id, dialog_id);
            
            // PHASE 0.24: Log BYE completion timing
            let completion_time = start_time.elapsed();
            tracing::info!("⏱️ BYE-TIMING: Session {} BYE completion took {:?}", session_id, completion_time);
            
            // Send SessionTerminated event for the UAC
            self.send_session_event(SessionEvent::SessionTerminated {
                session_id: session_id.clone(),
                reason: "Call terminated by local BYE".to_string(),
            }).await.unwrap_or_else(|e| {
                tracing::error!("❌ BYE-SUCCESS: Failed to send SessionTerminated event for {}: {}", session_id, e);
            });
            
            tracing::info!("✅ BYE-SUCCESS: Generated SessionTerminated event for UAC session {} in {:?}", 
                         session_id, start_time.elapsed());
            println!("✅ BYE-SUCCESS: Generated SessionTerminated event for UAC session {}", session_id);
            
            // NOW remove the dialog mapping after sending the event
            self.dialog_to_session.remove(&dialog_id);
            
            // Also clean up From URI mappings for this session
            self.untrack_from_uri_for_session(&session_id);
            
            // PHASE 0.24: Enhanced cleanup logging
            tracing::debug!("🧹 BYE-CLEANUP: Removed dialog mapping for {} (session: {})", dialog_id, session_id);
            
            // Log final BYE success summary
            tracing::info!("🎉 BYE-FINAL: Session {} termination completed successfully in {:?}", 
                         session_id, start_time.elapsed());
            
        } else {
            tracing::warn!("❌ BYE-SUCCESS: No session found for dialog {} - mapping may have been removed prematurely", dialog_id);
            println!("❌ BYE-SUCCESS: No session found for dialog {}", dialog_id);
            
            // PHASE 0.24: Still remove dialog if no session (cleanup)
            self.dialog_to_session.remove(&dialog_id);
            tracing::debug!("🧹 BYE-CLEANUP: Removed orphaned dialog mapping for {}", dialog_id);
        }
        
        Ok(())
    }
    
    /// Handle registration request coordination event
    async fn handle_registration_request(
        &self,
        transaction_id: rvoip_dialog_core::TransactionKey,
        from_uri: String,
        contact_uri: String,
        expires: u32,
    ) -> DialogResult<()> {
        tracing::info!("Registration request: {} -> {} (expires: {})", from_uri, contact_uri, expires);
        
        // Forward registration request to application via SessionEvent
        self.send_session_event(SessionEvent::RegistrationRequest {
            transaction_id: transaction_id.to_string(),
            from_uri,
            contact_uri,
            expires,
        }).await?;
        
        // The application (CallCenterEngine) will process this and send the response
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
            
            rvoip_sip_core::Method::Notify => {
                // Handle NOTIFY for REFER subscriptions - just acknowledge with 200 OK
                tracing::info!("Received NOTIFY for dialog {}, sending 200 OK", dialog_id);
                if let Err(e) = self.send_response(&transaction_id, 200, "OK").await {
                    tracing::error!("Failed to send 200 OK response to NOTIFY: {}", e);
                }
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
    pub async fn send_response(
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
        // Get the current state before updating
        let old_state = if let Ok(Some(session)) = self.registry.get_session(&session_id).await {
            session.state().clone()
        } else {
            return Err(DialogError::SessionNotFound { session_id: session_id.0 });
        };
        
        // Update the state using the internal registry
        self.registry.update_session_state(&session_id, new_state.clone()).await
            .map_err(|e| DialogError::Coordination {
                message: format!("Failed to update session state: {}", e),
            })?;
        
        // Send state changed event
        self.send_session_event(SessionEvent::StateChanged {
                session_id,
                old_state,
                new_state,
            }).await?;
        
        Ok(())
    }
    
    /// Send a session event
    /// Map session to dialog (for outgoing calls)
    pub fn map_session_to_dialog(&self, session_id: SessionId, dialog_id: DialogId, call_id: Option<String>) {
        self.session_to_dialog.insert(session_id.clone(), dialog_id.clone());
        self.dialog_to_session.insert(dialog_id, session_id.clone());
        
        // Also track Call-ID to session mapping if provided
        if let Some(cid) = call_id {
            self.callid_to_session.insert(cid, session_id);
        }
        
        tracing::info!("📍 Mapped session-to-dialog bidirectionally");
    }
    
    /// Track From URI for a session (for faster UAC-side correlation)
    pub fn track_from_uri(&self, session_id: SessionId, from_uri: &str) {
        self.fromuri_to_session.insert(from_uri.to_string(), session_id.clone());
        tracing::info!("Tracked From URI {} for session {}", from_uri, session_id);
    }
    
    pub fn untrack_from_uri_for_session(&self, session_id: &SessionId) {
        // Find and remove all From URI mappings for this session
        let uris_to_remove: Vec<String> = self.fromuri_to_session
            .iter()
            .filter(|entry| entry.value() == session_id)
            .map(|entry| entry.key().clone())
            .collect();
            
        for uri in uris_to_remove {
            self.fromuri_to_session.remove(&uri);
            tracing::debug!("Untracked From URI {} for session {}", uri, session_id);
        }
    }
    
    async fn send_session_event(&self, event: SessionEvent) -> DialogResult<()> {
        self.event_processor
            .publish_event(event)
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
        // Create a new transfer handler for the cloned instance
        let transfer_handler = Arc::new(TransferHandler::new(
            Arc::clone(&self.dialog_api),
            Arc::clone(&self.registry),
            Arc::clone(&self.dialog_to_session),
            Arc::clone(&self.session_to_dialog),
            Arc::clone(&self.event_processor),
        ));
        
        Self {
            dialog_api: Arc::clone(&self.dialog_api),
            registry: Arc::clone(&self.registry),
            handler: self.handler.clone(),
            event_processor: self.event_processor.clone(),
            dialog_to_session: Arc::clone(&self.dialog_to_session),
            session_to_dialog: Arc::clone(&self.session_to_dialog),
            incoming_sdp_offers: Arc::clone(&self.incoming_sdp_offers),
            callid_to_session: Arc::clone(&self.callid_to_session),
            fromtag_to_session: Arc::clone(&self.fromtag_to_session),
            fromuri_to_session: Arc::clone(&self.fromuri_to_session),
            transfer_handler,
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