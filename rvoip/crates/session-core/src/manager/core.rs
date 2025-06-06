//! Core SessionManager Implementation
//!
//! Contains the main SessionManager struct with core coordination logic.

use std::sync::Arc;
use tokio::sync::mpsc;
use dashmap::DashMap;
use crate::api::{
    types::{CallSession, SessionId, SessionStats, MediaInfo},
    handlers::CallHandler,
    builder::SessionManagerConfig,
};
use crate::errors::Result;
use super::{registry::SessionRegistry, events::SessionEventProcessor, cleanup::CleanupManager};

// Dialog-core integration (only layer we integrate with) - using UnifiedDialogApi
use rvoip_dialog_core::{
    api::unified::UnifiedDialogApi,
    events::SessionCoordinationEvent,
    DialogId, DialogError,
};
// Import header name constants
use rvoip_sip_core::types::headers::HeaderName;

/// Main SessionManager that coordinates all session operations
pub struct SessionManager {
    config: SessionManagerConfig,
    registry: Arc<SessionRegistry>,
    event_processor: Arc<SessionEventProcessor>,
    cleanup_manager: Arc<CleanupManager>,
    handler: Option<Arc<dyn CallHandler>>,
    dialog_api: Arc<UnifiedDialogApi>,
    session_events_tx: mpsc::Sender<SessionCoordinationEvent>,
    dialog_to_session: Arc<dashmap::DashMap<DialogId, SessionId>>,
}

impl std::fmt::Debug for SessionManager {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SessionManager")
            .field("config", &self.config)
            .field("registry", &self.registry)
            .field("event_processor", &self.event_processor)
            .field("cleanup_manager", &self.cleanup_manager)
            .field("handler", &self.handler.is_some())
            .field("dialog_api", &"<UnifiedDialogApi>")
            .field("dialog_to_session", &self.dialog_to_session.len())
            .finish()
    }
}

impl SessionManager {
    /// Create a new SessionManager with the given configuration  
    pub async fn new(
        config: SessionManagerConfig,
        handler: Option<Arc<dyn CallHandler>>,
        dialog_api: Arc<UnifiedDialogApi>,
    ) -> Result<Arc<Self>> {
        let registry = Arc::new(SessionRegistry::new());
        let event_processor = Arc::new(SessionEventProcessor::new());
        let cleanup_manager = Arc::new(CleanupManager::new());

        // Create session coordination channel
        let (session_events_tx, session_events_rx) = mpsc::channel(1000);
        
        // Set up dialog-to-session mapping
        let dialog_to_session = Arc::new(DashMap::new());

        let manager = Arc::new(Self {
            config,
            registry,
            event_processor,
            cleanup_manager,
            handler,
            dialog_api,
            session_events_tx,
            dialog_to_session,
        });

        // Initialize subsystems and coordination
        manager.initialize(session_events_rx).await?;

        Ok(manager)
    }

    /// Initialize the session manager and all subsystems
    async fn initialize(&self, mut session_events_rx: mpsc::Receiver<SessionCoordinationEvent>) -> Result<()> {
        // Set up session coordination with dialog-core
        println!("🔗 SETUP: Setting up session coordination with dialog-core");
        self.dialog_api.set_session_coordinator(self.session_events_tx.clone())
            .await
            .map_err(|e| crate::errors::SessionError::internal(&format!("Failed to set session coordinator: {}", e)))?;
        println!("✅ SETUP: Session coordination setup complete");

        // Spawn task to handle session coordination events
        println!("🎬 SPAWN: Starting session coordination event loop");
        let manager = self.clone();
        tokio::spawn(async move {
            println!("📡 EVENT LOOP: Session coordination event loop started");
            while let Some(event) = session_events_rx.recv().await {
                println!("📨 EVENT LOOP: Received session coordination event in background task");
                if let Err(e) = manager.handle_session_coordination_event(event).await {
                    tracing::error!("Error handling session coordination event: {}", e);
                }
            }
            println!("🏁 EVENT LOOP: Session coordination event loop ended");
        });

        tracing::info!("SessionManager initialized on port {}", self.config.sip_port);
        Ok(())
    }

    /// Start the session manager
    pub async fn start(&self) -> Result<()> {
        // Start dialog API
        self.dialog_api.start()
            .await
            .map_err(|e| crate::errors::SessionError::internal(&format!("Failed to start dialog API: {}", e)))?;
        
        self.event_processor.start().await?;
        self.cleanup_manager.start().await?;
        tracing::info!("SessionManager started");
        Ok(())
    }

    /// Stop the session manager
    pub async fn stop(&self) -> Result<()> {
        self.cleanup_manager.stop().await?;
        self.event_processor.stop().await?;
        
        // Stop dialog API
        self.dialog_api.stop()
            .await
            .map_err(|e| crate::errors::SessionError::internal(&format!("Failed to stop dialog API: {}", e)))?;
            
        tracing::info!("SessionManager stopped");
        Ok(())
    }

    /// Create an outgoing call session
    pub async fn create_outgoing_call(
        &self,
        from: &str,
        to: &str,
        sdp: Option<String>,
    ) -> Result<CallSession> {
        let session_id = SessionId::new();
        
        // Create SIP INVITE and dialog using dialog-core unified API
        let call_handle = self.dialog_api.make_call(from, to, sdp)
            .await
            .map_err(|e| crate::errors::SessionError::internal(&format!("Failed to create call via dialog-core: {}", e)))?;
        
        // Map dialog to session
        self.dialog_to_session.insert(call_handle.dialog().id().clone(), session_id.clone());
        
        let call = CallSession {
            id: session_id.clone(),
            from: from.to_string(),
            to: to.to_string(),
            state: crate::api::types::CallState::Initiating,
            started_at: Some(std::time::Instant::now()),
            manager: Arc::new(self.clone()),
        };

        // Register the session
        self.registry.register_session(session_id.clone(), call.clone()).await?;

        // Send session created event
        self.send_session_event(super::events::SessionEvent::SessionCreated {
            session_id: session_id.clone(),
            from: call.from.clone(),
            to: call.to.clone(),
            call_state: call.state.clone(),
        }).await?;

        tracing::info!("Created outgoing call: {} -> {}", from, to);
        Ok(call)
    }

    /// Accept an incoming call
    pub async fn accept_incoming_call(&self, session_id: &SessionId) -> Result<CallSession> {
        let call = self.registry.get_session(session_id).await?
            .ok_or_else(|| crate::errors::SessionError::session_not_found(&session_id.0))?;
        
        // Find the corresponding dialog
        let dialog_id = self.dialog_to_session.iter()
            .find_map(|entry| if entry.value() == session_id { Some(entry.key().clone()) } else { None })
            .ok_or_else(|| crate::errors::SessionError::session_not_found(&session_id.0))?;
        
        // Send 200 OK response via dialog-core (this would be done by the call handler normally)
        tracing::info!("Accepted incoming call: {} (dialog: {})", session_id, dialog_id);
        Ok(call)
    }

    /// Hold a session
    pub async fn hold_session(&self, session_id: &SessionId) -> Result<()> {
        let dialog_id = self.get_dialog_id_for_session(session_id)?;
        
        // Send re-INVITE with hold SDP via dialog-core unified API
        let _tx_key = self.dialog_api.send_update(&dialog_id, Some("SDP with hold attributes".to_string()))
            .await
            .map_err(|e| crate::errors::SessionError::internal(&format!("Failed to hold session: {}", e)))?;
            
        tracing::info!("Holding session: {}", session_id);
        Ok(())
    }

    /// Resume a session from hold
    pub async fn resume_session(&self, session_id: &SessionId) -> Result<()> {
        let dialog_id = self.get_dialog_id_for_session(session_id)?;
        
        // Send re-INVITE with active SDP via dialog-core unified API
        let _tx_key = self.dialog_api.send_update(&dialog_id, Some("SDP with active media".to_string()))
            .await
            .map_err(|e| crate::errors::SessionError::internal(&format!("Failed to resume session: {}", e)))?;
            
        tracing::info!("Resuming session: {}", session_id);
        Ok(())
    }

    /// Transfer a session to another destination
    pub async fn transfer_session(&self, session_id: &SessionId, target: &str) -> Result<()> {
        let dialog_id = self.get_dialog_id_for_session(session_id)?;
        
        // Send REFER request via dialog-core unified API
        let _tx_key = self.dialog_api.send_refer(&dialog_id, target.to_string(), None)
            .await
            .map_err(|e| crate::errors::SessionError::internal(&format!("Failed to transfer session: {}", e)))?;
            
        tracing::info!("Transferring session {} to {}", session_id, target);
        Ok(())
    }

    /// Terminate a session
    pub async fn terminate_session(&self, session_id: &SessionId) -> Result<()> {
        let dialog_id = self.get_dialog_id_for_session(session_id)?;
        
        // Send BYE request via dialog-core unified API
        let _tx_key = self.dialog_api.send_bye(&dialog_id)
            .await
            .map_err(|e| crate::errors::SessionError::internal(&format!("Failed to terminate session: {}", e)))?;
            
        // Remove the session from registry
        self.registry.unregister_session(session_id).await?;
        self.dialog_to_session.remove(&dialog_id);
        
        tracing::info!("Terminated session: {}", session_id);
        Ok(())
    }

    /// Send DTMF tones
    pub async fn send_dtmf(&self, session_id: &SessionId, digits: &str) -> Result<()> {
        let dialog_id = self.get_dialog_id_for_session(session_id)?;
        
        // Send INFO request with DTMF payload via dialog-core unified API
        let _tx_key = self.dialog_api.send_info(&dialog_id, format!("DTMF: {}", digits))
            .await
            .map_err(|e| crate::errors::SessionError::internal(&format!("Failed to send DTMF: {}", e)))?;
            
        tracing::info!("Sending DTMF {} to session {}", digits, session_id);
        Ok(())
    }
    
    /// Get dialog ID for a session ID
    fn get_dialog_id_for_session(&self, session_id: &SessionId) -> Result<DialogId> {
        self.dialog_to_session.iter()
            .find_map(|entry| if entry.value() == session_id { Some(entry.key().clone()) } else { None })
            .ok_or_else(|| crate::errors::SessionError::session_not_found(&session_id.0))
    }

    /// Mute/unmute a session
    pub async fn mute_session(&self, session_id: &SessionId, muted: bool) -> Result<()> {
        // TODO: Delegate to media-core for actual media stream control
        // This would update the media streams without SIP signaling
        tracing::info!("Muting session {}: {}", session_id, muted);
        Ok(())
    }

    /// Get media information for a session
    pub async fn get_media_info(&self, session_id: &SessionId) -> Result<MediaInfo> {
        // TODO: Delegate to media-core for actual media info
        // This would get the current SDP, ports, codecs from media-core
        Ok(MediaInfo {
            local_sdp: None,
            remote_sdp: None,
            local_rtp_port: None,
            remote_rtp_port: None,
            codec: None,
        })
    }

    /// Update media for a session
    pub async fn update_media(&self, session_id: &SessionId, sdp: &str) -> Result<()> {
        let dialog_id = self.get_dialog_id_for_session(session_id)?;
        
        // Send re-INVITE with new SDP via dialog-core unified API
        let _tx_key = self.dialog_api.send_update(&dialog_id, Some(sdp.to_string()))
            .await
            .map_err(|e| crate::errors::SessionError::internal(&format!("Failed to update media: {}", e)))?;
            
        tracing::info!("Updating media for session {}", session_id);
        Ok(())
    }

    /// Get statistics about active sessions
    pub async fn get_stats(&self) -> Result<SessionStats> {
        self.registry.get_stats().await
    }

    /// List all active sessions
    pub async fn list_active_sessions(&self) -> Result<Vec<SessionId>> {
        self.registry.list_active_sessions().await
    }

    /// Find a session by ID
    pub async fn find_session(&self, session_id: &SessionId) -> Result<Option<CallSession>> {
        self.registry.get_session(session_id).await
    }

    /// Get the call handler
    pub fn get_handler(&self) -> Option<&Arc<dyn CallHandler>> {
        self.handler.as_ref()
    }

    /// Get the event processor (for testing)
    pub fn get_event_processor(&self) -> &Arc<SessionEventProcessor> {
        &self.event_processor
    }

    /// Get the actual bound address (for testing and discovery)
    pub fn get_bound_address(&self) -> std::net::SocketAddr {
        self.dialog_api.config().local_address()
    }

    /// Send a session event
    async fn send_session_event(&self, event: super::events::SessionEvent) -> Result<()> {
        self.event_processor.publish_event(event).await
    }

    /// Handle session coordination events from dialog-core
    async fn handle_session_coordination_event(&self, event: SessionCoordinationEvent) -> Result<()> {
        println!("🎪 SESSION COORDINATION: Received event: {:?}", event);
        match event {
            SessionCoordinationEvent::IncomingCall { dialog_id, transaction_id, request, source } => {
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
                    state: crate::api::types::CallState::Ringing,
                    started_at: Some(std::time::Instant::now()),
                    manager: Arc::new(self.clone()),
                };
                
                self.registry.register_session(session_id.clone(), call_session.clone()).await?;
                
                // Send session created event
                self.send_session_event(super::events::SessionEvent::SessionCreated {
                    session_id: session_id.clone(),
                    from: call_session.from.clone(),
                    to: call_session.to.clone(),
                    call_state: call_session.state.clone(),
                }).await?;
                
                // Handle the call with the configured handler
                if let Some(handler) = &self.handler {
                    let incoming_call = crate::api::types::IncomingCall {
                        id: session_id.clone(),
                        from: from_uri,
                        to: to_uri,
                        sdp: Some(String::from_utf8_lossy(request.body()).to_string()).filter(|s| !s.is_empty()),
                        headers: std::collections::HashMap::new(), // TODO: Extract relevant headers
                        received_at: std::time::Instant::now(),
                    };
                    
                    let decision = handler.on_incoming_call(incoming_call).await;
                    
                    // Act on the call decision
                    match decision {
                        crate::api::types::CallDecision::Accept => {
                            // Get the call handle for this dialog and answer it
                            if let Ok(call_handle) = self.dialog_api.get_call_handle(&dialog_id).await {
                                if let Err(e) = call_handle.answer(None).await {
                                    tracing::error!("Failed to answer incoming call for session {}: {}", session_id, e);
                                    
                                    // Update session state to failed
                                    if let Ok(Some(mut call)) = self.registry.get_session(&session_id).await {
                                        let old_state = call.state.clone();
                                        call.state = crate::api::types::CallState::Failed(format!("Answer failed: {}", e));
                                        let _ = self.registry.register_session(session_id.clone(), call).await;
                                        
                                        // Send state changed event
                                        let _ = self.send_session_event(super::events::SessionEvent::StateChanged {
                                            session_id: session_id.clone(),
                                            old_state,
                                            new_state: crate::api::types::CallState::Failed(format!("Answer failed: {}", e)),
                                        }).await;
                                    }
                                } else {
                                    tracing::info!("Successfully answered incoming call for session {}", session_id);
                                }
                            } else {
                                tracing::error!("Failed to get call handle for dialog {} to answer call", dialog_id);
                            }
                        },
                        
                        crate::api::types::CallDecision::Reject(reason) => {
                            tracing::info!("Rejecting incoming call for session {}: {}", session_id, reason);
                            
                            // Get the call handle and reject it
                                                         if let Ok(call_handle) = self.dialog_api.get_call_handle(&dialog_id).await {
                                 if let Err(e) = call_handle.reject(rvoip_sip_core::StatusCode::BusyHere, Some(reason.clone())).await {
                                     tracing::error!("Failed to reject incoming call for session {}: {}", session_id, e);
                                 }
                             }
                            
                                                         // Update session state to failed/rejected
                             if let Ok(Some(mut call)) = self.registry.get_session(&session_id).await {
                                 let old_state = call.state.clone();
                                 call.state = crate::api::types::CallState::Failed(reason);
                                 let new_state = call.state.clone();
                                 let _ = self.registry.register_session(session_id.clone(), call).await;
                                 
                                 // Send state changed event
                                 let _ = self.send_session_event(super::events::SessionEvent::StateChanged {
                                     session_id: session_id.clone(),
                                     old_state,
                                     new_state,
                                 }).await;
                             }
                        },
                        
                        crate::api::types::CallDecision::Defer => {
                            tracing::info!("Deferring incoming call for session {} (e.g., added to queue)", session_id);
                            // Call remains in Ringing state for manual acceptance later
                        },
                        
                        crate::api::types::CallDecision::Forward(target) => {
                            tracing::info!("Forwarding incoming call for session {} to {}", session_id, target);
                            // TODO: Implement call forwarding via dialog-core
                            // For now, treat as rejection
                            if let Ok(call_handle) = self.dialog_api.get_call_handle(&dialog_id).await {
                                if let Err(e) = call_handle.reject(rvoip_sip_core::StatusCode::MovedTemporarily, Some(format!("Forwarded to {}", target))).await {
                                    tracing::error!("Failed to forward incoming call for session {}: {}", session_id, e);
                                }
                            }
                        },
                    }
                }
            },
            
            SessionCoordinationEvent::ResponseReceived { dialog_id, response, transaction_id } => {
                println!("🎯 SESSION COORDINATION: Received response {} for dialog {}", response.status_code(), dialog_id);
                
                // Check if this is a 200 OK response to an INVITE that needs an ACK
                if response.status_code() == 200 && transaction_id.to_string().contains("INVITE") && transaction_id.to_string().contains("client") {
                    println!("🚀 SESSION COORDINATION: This is a 200 OK to INVITE - sending automatic ACK");
                    
                    // Send ACK for 2xx response using the proper dialog-core API
                    // We need to access the dialog manager directly since CallHandle doesn't expose ACK
                    if let Ok(dialog_handle) = self.dialog_api.get_dialog_handle(&dialog_id).await {
                        // Get the underlying dialog manager from the unified API
                        // We'll call the send_ack_for_2xx_response method directly
                        match self.dialog_api.send_ack_for_2xx_response(&dialog_id, &transaction_id, &response).await {
                            Ok(_) => {
                                println!("✅ SESSION COORDINATION: Successfully sent ACK for 200 OK response");
                                tracing::info!("ACK sent successfully for dialog {} transaction {}", dialog_id, transaction_id);
                            },
                            Err(e) => {
                                println!("❌ SESSION COORDINATION: Failed to send ACK: {}", e);
                                tracing::error!("Failed to send ACK for dialog {} transaction {}: {}", dialog_id, transaction_id, e);
                            }
                        }
                    }
                }
                
                // Continue with other response processing...
                tracing::debug!("Response {} received for dialog {}", response.status_code(), dialog_id);
            },
            
            SessionCoordinationEvent::CallAnswered { dialog_id, session_answer } => {
                if let Some(session_id_ref) = self.dialog_to_session.get(&dialog_id) {
                    let session_id = session_id_ref.value().clone();
                    tracing::info!("Call answered for session {}: {}", session_id, dialog_id);
                    
                    // Update call state to Active
                    if let Ok(Some(mut call)) = self.registry.get_session(&session_id).await {
                        let old_state = call.state.clone();
                        call.state = crate::api::types::CallState::Active;
                        self.registry.register_session(session_id.clone(), call).await?;
                        
                        // Send state changed event
                        self.send_session_event(super::events::SessionEvent::StateChanged {
                            session_id: session_id.clone(),
                            old_state,
                            new_state: crate::api::types::CallState::Active,
                        }).await?;
                    }
                }
            },
            
            SessionCoordinationEvent::CallTerminated { dialog_id, reason } => {
                if let Some((_, session_id)) = self.dialog_to_session.remove(&dialog_id) {
                    tracing::info!("Call terminated for session {}: {} - {}", session_id, dialog_id, reason);
                    
                    // Send session terminated event
                    self.send_session_event(super::events::SessionEvent::SessionTerminated {
                        session_id: session_id.clone(),
                        reason: reason.clone(),
                    }).await?;
                    
                    self.registry.unregister_session(&session_id).await?;
                }
            },
            
            SessionCoordinationEvent::RegistrationRequest { transaction_id, from_uri, contact_uri, expires } => {
                tracing::info!("Registration request: {} -> {} (expires: {})", from_uri, contact_uri, expires);
                // Handle registration - for now just log it
                // In a real implementation, this would update a registration database
            },
            
            // Handle other session coordination events
            _ => {
                tracing::debug!("Unhandled session coordination event: {:?}", event);
                // TODO: Handle other events as needed
            },
        }
        
        Ok(())
    }
}

impl Clone for SessionManager {
    fn clone(&self) -> Self {
        Self {
            config: self.config.clone(),
            registry: Arc::clone(&self.registry),
            event_processor: Arc::clone(&self.event_processor),
            cleanup_manager: Arc::clone(&self.cleanup_manager),
            handler: self.handler.clone(),
            dialog_api: Arc::clone(&self.dialog_api),
            session_events_tx: self.session_events_tx.clone(),
            dialog_to_session: Arc::clone(&self.dialog_to_session),
        }
    }
} 