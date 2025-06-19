//! Event handling implementation for SessionCoordinator

use std::sync::Arc;
use tokio::sync::mpsc;
use crate::api::types::{SessionId, CallState};
use crate::errors::{Result, SessionError};
use crate::manager::events::SessionEvent;
use super::SessionCoordinator;

impl SessionCoordinator {
    /// Main event loop that handles all session events
    pub(crate) async fn run_event_loop(self: Arc<Self>, mut event_rx: mpsc::Receiver<SessionEvent>) {
        tracing::info!("Starting main coordinator event loop");

        while let Some(event) = event_rx.recv().await {
            if let Err(e) = self.handle_event(event).await {
                tracing::error!("Error handling event: {}", e);
            }
        }

        tracing::info!("Main coordinator event loop ended");
    }

    /// Handle a session event
    async fn handle_event(&self, event: SessionEvent) -> Result<()> {
        println!("🎯 COORDINATOR: Handling event: {:?}", event);
        tracing::debug!("Handling event: {:?}", event);

        // Publish event to subscribers BEFORE internal processing
        // This ensures subscribers see events in real-time, even if processing takes time
        if let Err(e) = self.event_processor.publish_event(event.clone()).await {
            tracing::error!("Failed to publish event to subscribers: {}", e);
            // Continue processing even if publishing fails
        }

        match event {
            SessionEvent::SessionCreated { session_id, from, to, call_state } => {
                self.handle_session_created(session_id, from, to, call_state).await?;
            }
            
            SessionEvent::StateChanged { session_id, old_state, new_state } => {
                self.handle_state_changed(session_id, old_state, new_state).await?;
            }
            
            SessionEvent::SessionTerminated { session_id, reason } => {
                println!("🎯 COORDINATOR: Matched SessionTerminated event for {} - {}", session_id, reason);
                self.handle_session_terminated(session_id, reason).await?;
            }
            
            SessionEvent::MediaEvent { session_id, event } => {
                self.handle_media_event(session_id, event).await?;
            }
            
            SessionEvent::SdpEvent { session_id, event_type, sdp } => {
                self.handle_sdp_event(session_id, event_type, sdp).await?;
            }
            
            SessionEvent::SdpNegotiationRequested { session_id, role, local_sdp, remote_sdp } => {
                self.handle_sdp_negotiation_request(session_id, role, local_sdp, remote_sdp).await?;
            }
            
            SessionEvent::RegistrationRequest { transaction_id, from_uri, contact_uri, expires } => {
                self.handle_registration_request(transaction_id, from_uri, contact_uri, expires).await?;
            }
            
            _ => {
                tracing::debug!("Unhandled event type");
            }
        }

        Ok(())
    }

    /// Handle session created event
    async fn handle_session_created(
        &self,
        session_id: SessionId,
        _from: String,
        _to: String,
        call_state: CallState,
    ) -> Result<()> {
        tracing::info!("Session {} created with state {:?}", session_id, call_state);

        // Media is created later when session becomes active
        match call_state {
            CallState::Ringing | CallState::Initiating => {
                tracing::debug!("Session {} in early state, deferring media setup", session_id);
            }
            CallState::Active => {
                tracing::warn!("Session {} created in Active state, starting media", session_id);
                self.start_media_session(&session_id).await?;
            }
            _ => {}
        }

        Ok(())
    }

    /// Handle session state change
    async fn handle_state_changed(
        &self,
        session_id: SessionId,
        old_state: CallState,
        new_state: CallState,
    ) -> Result<()> {
        tracing::info!("Session {} state changed: {:?} -> {:?}", session_id, old_state, new_state);

        match (old_state, new_state.clone()) {
            // Call becomes active
            (CallState::Ringing, CallState::Active) |
            (CallState::Initiating, CallState::Active) => {
                self.start_media_session(&session_id).await?;
                
                // Notify handler that call is established
                if let Some(handler) = &self.handler {
                    if let Ok(Some(session)) = self.registry.get_session(&session_id).await {
                        // Get SDP information if available
                        let media_info = self.media_manager.get_media_info(&session_id).await.ok().flatten();
                        
                        tracing::info!("Media info available: {}", media_info.is_some());
                        if let Some(ref info) = media_info {
                            tracing::info!("Local SDP length: {:?}", info.local_sdp.as_ref().map(|s| s.len()));
                            tracing::info!("Remote SDP length: {:?}", info.remote_sdp.as_ref().map(|s| s.len()));
                        }
                        
                        let local_sdp = media_info.as_ref().and_then(|m| m.local_sdp.clone());
                        let remote_sdp = media_info.as_ref().and_then(|m| m.remote_sdp.clone());
                        
                        tracing::info!("Notifying handler about call {} establishment", session_id);
                        handler.on_call_established(session, local_sdp, remote_sdp).await;
                    }
                }
            }
            
            // Call goes on hold
            (CallState::Active, CallState::OnHold) => {
                self.media_coordinator.on_session_hold(&session_id).await
                    .map_err(|e| SessionError::internal(&format!("Failed to hold media: {}", e)))?;
            }
            
            // Call resumes
            (CallState::OnHold, CallState::Active) => {
                self.media_coordinator.on_session_resume(&session_id).await
                    .map_err(|e| SessionError::internal(&format!("Failed to resume media: {}", e)))?;
            }
            
            // Call ends
            (_, CallState::Failed(_)) |
            (_, CallState::Terminated) => {
                self.stop_media_session(&session_id).await?;
            }
            
            _ => {}
        }

        Ok(())
    }

    /// Handle session terminated event
    async fn handle_session_terminated(
        &self,
        session_id: SessionId,
        reason: String,
    ) -> Result<()> {
        println!("🔴 COORDINATOR: handle_session_terminated called for session {} with reason: {}", session_id, reason);
        tracing::info!("Session {} terminated: {}", session_id, reason);

        // Stop media
        self.stop_media_session(&session_id).await?;

        // Update session state to Terminated before notifying handler
        let mut session_for_handler = None;
        if let Ok(Some(mut session)) = self.registry.get_session(&session_id).await {
            let old_state = session.state.clone();
            session.state = CallState::Terminated;
            
            // Store the session for handler notification
            session_for_handler = Some(session.clone());
            
            // Update the session in registry
            if let Err(e) = self.registry.register_session(session_id.clone(), session).await {
                tracing::error!("Failed to update session to Terminated state: {}", e);
            } else {
                // Emit state change event
                let _ = self.event_tx.send(SessionEvent::StateChanged {
                    session_id: session_id.clone(),
                    old_state,
                    new_state: CallState::Terminated,
                }).await;
            }
        }

        // Notify handler
        if let Some(handler) = &self.handler {
            println!("🔔 COORDINATOR: Handler exists, checking for session {}", session_id);
            if let Some(session) = session_for_handler {
                println!("✅ COORDINATOR: Found session {}, calling handler.on_call_ended", session_id);
                tracing::info!("Notifying handler about session {} termination", session_id);
                handler.on_call_ended(session, &reason).await;
            } else {
                println!("❌ COORDINATOR: Session {} not found in registry", session_id);
            }
        } else {
            println!("⚠️ COORDINATOR: No handler configured");
        }

        // Don't unregister immediately - let cleanup handle it later
        // This allows tests and other components to verify the Terminated state
        // self.registry.unregister_session(&session_id).await?;

        Ok(())
    }

    /// Handle media event
    async fn handle_media_event(
        &self,
        session_id: SessionId,
        event: String,
    ) -> Result<()> {
        tracing::debug!("Media event for session {}: {}", session_id, event);

        match event.as_str() {
            "rfc_compliant_media_creation_uac" | "rfc_compliant_media_creation_uas" => {
                tracing::info!("Media creation event for {}: {}", session_id, event);
                
                // Just update session state to Active - the state change handler will create media
                if let Ok(Some(mut session)) = self.registry.get_session(&session_id).await {
                    let old_state = session.state.clone();
                    
                    // Only update if not already Active
                    if !matches!(old_state, CallState::Active) {
                        session.state = CallState::Active;
                        
                        if let Err(e) = self.registry.register_session(session_id.clone(), session).await {
                            tracing::error!("Failed to update session state: {}", e);
                        } else {
                            // Publish state change event - this will trigger media creation
                            let _ = self.event_tx.send(SessionEvent::StateChanged {
                                session_id,
                                old_state,
                                new_state: CallState::Active,
                            }).await;
                        }
                    } else {
                        tracing::debug!("Session {} already Active, skipping state update", session_id);
                    }
                }
            }
            _ => {}
        }

        Ok(())
    }

    /// Handle SDP event
    async fn handle_sdp_event(
        &self,
        session_id: SessionId,
        event_type: String,
        sdp: String,
    ) -> Result<()> {
        tracing::debug!("SDP event for session {}: {}", session_id, event_type);

        match event_type.as_str() {
            "remote_sdp_answer" => {
                // For UAC: we sent offer, received answer - negotiate
                if let Ok(Some(session)) = self.registry.get_session(&session_id).await {
                    // Get our offer from media manager
                    if let Ok(Some(media_info)) = self.media_manager.get_media_info(&session_id).await {
                        if let Some(our_offer) = media_info.local_sdp {
                            tracing::info!("Negotiating SDP as UAC for session {}", session_id);
                            match self.negotiate_sdp_as_uac(&session_id, &our_offer, &sdp).await {
                                Ok(negotiated) => {
                                    tracing::info!("SDP negotiation successful: codec={}, local={}, remote={}", 
                                        negotiated.codec, negotiated.local_addr, negotiated.remote_addr);
                                }
                                Err(e) => {
                                    tracing::error!("SDP negotiation failed: {}", e);
                                }
                            }
                        }
                    }
                }
            }
            "final_negotiated_sdp" => {
                if let Ok(Some(_)) = self.media_manager.get_media_info(&session_id).await {
                    if let Err(e) = self.media_manager.update_media_session(&session_id, &sdp).await {
                        tracing::error!("Failed to update media session with SDP: {}", e);
                    }
                }
            }
            _ => {}
        }

        Ok(())
    }

    /// Handle registration request
    async fn handle_registration_request(
        &self,
        _transaction_id: String,
        from_uri: String,
        contact_uri: String,
        expires: u32,
    ) -> Result<()> {
        tracing::info!("REGISTER request forwarded to application: {} -> {} (expires: {})", from_uri, contact_uri, expires);
        
        // Forward to application handler if available
        // In a complete implementation, the CallCenterEngine would subscribe to these events
        // and process them with its SipRegistrar
        
        // For now, we just log it - the application should subscribe to SessionEvent::RegistrationRequest
        // and handle it appropriately by sending a response back through dialog-core
        
        Ok(())
    }

    /// Handle SDP negotiation request
    async fn handle_sdp_negotiation_request(
        &self,
        session_id: SessionId,
        role: String,
        local_sdp: Option<String>,
        remote_sdp: Option<String>,
    ) -> Result<()> {
        tracing::info!("SDP negotiation requested for session {} as {}", session_id, role);
        
        match role.as_str() {
            "uas" => {
                // We're the UAS - received offer, need to generate answer
                if let Some(their_offer) = remote_sdp {
                    match self.negotiate_sdp_as_uas(&session_id, &their_offer).await {
                        Ok((our_answer, negotiated)) => {
                            tracing::info!("SDP negotiation as UAS successful: codec={}, local={}, remote={}", 
                                negotiated.codec, negotiated.local_addr, negotiated.remote_addr);
                            
                            // Send event with the generated answer
                            let _ = self.event_tx.send(SessionEvent::SdpEvent {
                                session_id,
                                event_type: "generated_sdp_answer".to_string(),
                                sdp: our_answer,
                            }).await;
                        }
                        Err(e) => {
                            tracing::error!("SDP negotiation as UAS failed: {}", e);
                        }
                    }
                }
            }
            "uac" => {
                // We're the UAC - sent offer, received answer
                if let (Some(our_offer), Some(their_answer)) = (local_sdp, remote_sdp) {
                    match self.negotiate_sdp_as_uac(&session_id, &our_offer, &their_answer).await {
                        Ok(negotiated) => {
                            tracing::info!("SDP negotiation as UAC successful: codec={}, local={}, remote={}", 
                                negotiated.codec, negotiated.local_addr, negotiated.remote_addr);
                        }
                        Err(e) => {
                            tracing::error!("SDP negotiation as UAC failed: {}", e);
                        }
                    }
                }
            }
            _ => {
                tracing::warn!("Unknown SDP negotiation role: {}", role);
            }
        }
        
        Ok(())
    }
} 