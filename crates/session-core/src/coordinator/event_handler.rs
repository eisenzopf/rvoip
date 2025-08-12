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
            
            SessionEvent::DetailedStateChange { session_id, old_state, new_state, reason, .. } => {
                // Handle the enhanced state change event
                self.handle_state_changed(session_id.clone(), old_state.clone(), new_state.clone()).await?;
                
                // Also notify the CallHandler about the state change
                if let Some(handler) = &self.handler {
                    handler.on_call_state_changed(&session_id, &old_state, &new_state, reason.as_deref()).await;
                }
            }
            
            SessionEvent::SessionTerminating { session_id, reason } => {
                println!("🎯 COORDINATOR: Matched SessionTerminating event (Phase 1) for {} - {}", session_id, reason);
                self.handle_session_terminating(session_id, reason).await?;
            }
            
            SessionEvent::SessionTerminated { session_id, reason } => {
                println!("🎯 COORDINATOR: Matched SessionTerminated event (Phase 2) for {} - {}", session_id, reason);
                self.handle_session_terminated(session_id, reason).await?;
            }
            
            SessionEvent::CleanupConfirmation { session_id, layer } => {
                println!("🧹 COORDINATOR: Cleanup confirmation from {} for session {}", layer, session_id);
                self.handle_cleanup_confirmation(session_id, layer).await?;
            }
            
            SessionEvent::MediaEvent { session_id, event } => {
                self.handle_media_event(session_id, event).await?;
            }
            
            SessionEvent::MediaQuality { session_id, mos_score, packet_loss, alert_level, .. } => {
                // Notify handler about media quality
                if let Some(handler) = &self.handler {
                    handler.on_media_quality(&session_id, mos_score, packet_loss, alert_level).await;
                }
            }
            
            SessionEvent::DtmfDigit { session_id, digit, duration_ms, .. } => {
                // Notify handler about DTMF digit
                if let Some(handler) = &self.handler {
                    handler.on_dtmf(&session_id, digit, duration_ms).await;
                }
            }
            
            SessionEvent::MediaFlowChange { session_id, direction, active, codec } => {
                // Notify handler about media flow change
                if let Some(handler) = &self.handler {
                    handler.on_media_flow(&session_id, direction, active, &codec).await;
                }
            }
            
            SessionEvent::Warning { session_id, category, message } => {
                // Notify handler about warning
                if let Some(handler) = &self.handler {
                    handler.on_warning(session_id.as_ref(), category, &message).await;
                }
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
        println!("🔄 handle_state_changed called: {} {:?} -> {:?}", session_id, old_state, new_state);

        match (old_state, new_state.clone()) {
            // Call becomes active
            (CallState::Ringing, CallState::Active) |
            (CallState::Initiating, CallState::Active) => {
                println!("📞 Starting media session for newly active call: {}", session_id);
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
                        
                        // Store SDP in the registry so hold/resume can access it
                        if local_sdp.is_some() || remote_sdp.is_some() {
                            if let Err(e) = self.registry.update_session_sdp(&session_id, local_sdp.clone(), remote_sdp.clone()).await {
                                tracing::error!("Failed to store SDP in registry: {}", e);
                            } else {
                                tracing::info!("Stored SDP in registry for session {}", session_id);
                            }
                        }
                        
                        tracing::info!("Notifying handler about call {} establishment", session_id);
                        handler.on_call_established(session.as_call_session().clone(), local_sdp, remote_sdp).await;
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

    /// Handle session terminating event (Phase 1 - prepare for cleanup)
    async fn handle_session_terminating(
        &self,
        session_id: SessionId,
        reason: String,
    ) -> Result<()> {
        println!("🟡 COORDINATOR: handle_session_terminating called for session {} (Phase 1) with reason: {}", session_id, reason);
        tracing::info!("Session {} terminating (Phase 1): {}", session_id, reason);

        // Update session state to Terminating
        if let Ok(Some(session)) = self.registry.get_session(&session_id).await {
            let old_state = session.state().clone();
            
            // Update the session state to Terminating
            if let Err(e) = self.registry.update_session_state(&session_id, CallState::Terminating).await {
                tracing::error!("Failed to update session to Terminating state: {}", e);
            } else {
                // Emit state change event
                let _ = self.event_tx.send(SessionEvent::StateChanged {
                    session_id: session_id.clone(),
                    old_state: old_state.clone(),
                    new_state: CallState::Terminating,
                }).await;
            }
            
            // Notify handler about terminating state (Phase 1)
            if let Some(handler) = &self.handler {
                let call_session = session.as_call_session().clone();
                handler.on_call_state_changed(&session_id, &old_state, &CallState::Terminating, Some(&reason)).await;
            }
        }
        
        // Start tracking cleanup
        use super::coordinator::CleanupTracker;
        use std::time::Instant;
        
        let mut pending_cleanups = self.pending_cleanups.lock().await;
        pending_cleanups.insert(session_id.clone(), CleanupTracker {
            media_done: false,
            client_done: false,
            started_at: Instant::now(),
            reason: reason.clone(),
        });
        
        // Stop media gracefully
        self.stop_media_session(&session_id).await?;
        
        Ok(())
    }

    /// Handle cleanup confirmation from a layer
    async fn handle_cleanup_confirmation(
        &self,
        session_id: SessionId,
        layer: String,
    ) -> Result<()> {
        println!("🧹 COORDINATOR: handle_cleanup_confirmation called for session {} from layer {}", session_id, layer);
        tracing::info!("Cleanup confirmation from {} for session {}", layer, session_id);
        
        use super::coordinator::CleanupLayer;
        use std::time::Duration;
        
        let mut pending_cleanups = self.pending_cleanups.lock().await;
        
        if let Some(tracker) = pending_cleanups.get_mut(&session_id) {
            // Mark the appropriate layer as done
            match layer.as_str() {
                "Media" => {
                    tracker.media_done = true;
                    println!("✓ Media cleanup complete for session {}", session_id);
                }
                "Client" => {
                    tracker.client_done = true;
                    println!("✓ Client cleanup complete for session {}", session_id);
                }
                layer => {
                    tracing::warn!("Unknown cleanup layer: {}", layer);
                }
            }
            
            // Check if all cleanup is complete or if we've timed out
            let elapsed = tracker.started_at.elapsed();
            let timeout = Duration::from_secs(5);
            // For now, only require media cleanup since client cleanup is not being sent
            // TODO: Implement proper client cleanup for dialog-core
            let all_done = tracker.media_done; // Only checking media for now
            let timed_out = elapsed > timeout;
            
            if all_done || timed_out {
                if timed_out {
                    tracing::warn!("Cleanup timeout for session {} after {:?}", session_id, elapsed);
                } else {
                    tracing::info!("All cleanup complete for session {} in {:?}", session_id, elapsed);
                }
                
                // Remove from pending cleanups
                let reason = tracker.reason.clone();
                pending_cleanups.remove(&session_id);
                
                // Trigger Phase 2 - final termination
                println!("🔴 Triggering Phase 2 termination for session {}", session_id);
                let _ = self.event_tx.send(SessionEvent::SessionTerminated {
                    session_id: session_id.clone(),
                    reason,
                }).await;
            }
        } else {
            tracing::warn!("Received cleanup confirmation for unknown session: {}", session_id);
        }
        
        Ok(())
    }

    /// Handle session terminated event (Phase 2 - final cleanup)
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
        let mut call_session_for_handler = None;
        if let Ok(Some(session)) = self.registry.get_session(&session_id).await {
            let old_state = session.state().clone();
            
            // Update the session state in registry FIRST
            if let Err(e) = self.registry.update_session_state(&session_id, CallState::Terminated).await {
                tracing::error!("Failed to update session to Terminated state: {}", e);
            } else {
                // Emit state change event
                let _ = self.event_tx.send(SessionEvent::StateChanged {
                    session_id: session_id.clone(),
                    old_state,
                    new_state: CallState::Terminated,
                }).await;
            }
            
            // Now get the updated session with Terminated state for handler notification
            if let Ok(Some(updated_session)) = self.registry.get_session(&session_id).await {
                call_session_for_handler = Some(updated_session.as_call_session().clone());
            }
        }

        // Notify handler
        if let Some(handler) = &self.handler {
            println!("🔔 COORDINATOR: Handler exists, checking for session {}", session_id);
            if let Some(call_session) = call_session_for_handler {
                println!("✅ COORDINATOR: Found session {}, calling handler.on_call_ended", session_id);
                tracing::info!("Notifying handler about session {} termination", session_id);
                handler.on_call_ended(call_session, &reason).await;
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
                if let Ok(Some(session)) = self.registry.get_session(&session_id).await {
                    let old_state = session.state().clone();
                    
                    // Only update if not already Active
                    if !matches!(old_state, CallState::Active) {
                        if let Err(e) = self.registry.update_session_state(&session_id, CallState::Active).await {
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
                    // Get our offer from media manager (if media session exists)
                    let media_info = self.media_manager.get_media_info(&session_id).await.ok().flatten();
                    
                    if let Some(media_info) = media_info {
                        // Media session exists - normal flow
                        if let Some(our_offer) = media_info.local_sdp {
                            tracing::info!("Negotiating SDP as UAC for session {}", session_id);
                            match self.negotiate_sdp_as_uac(&session_id, &our_offer, &sdp).await {
                                Ok(negotiated) => {
                                    tracing::info!("SDP negotiation successful: codec={}, local={}, remote={}", 
                                        negotiated.codec, negotiated.local_addr, negotiated.remote_addr);
                                    
                                    // Update the media session with the remote SDP
                                    // This stores the SDP and configures the remote RTP endpoint
                                    if let Err(e) = self.media_manager.update_media_session(&session_id, &sdp).await {
                                        tracing::error!("Failed to update media session with remote SDP: {}", e);
                                    } else {
                                        tracing::info!("Updated media session with remote SDP for session {}", session_id);
                                        
                                        // Store the negotiated SDP in the registry
                                        if let Err(e) = self.registry.update_session_sdp(&session_id, Some(our_offer.clone()), Some(sdp.clone())).await {
                                            tracing::error!("Failed to store negotiated SDP in registry: {}", e);
                                        } else {
                                            tracing::info!("Stored negotiated SDP in registry for session {}", session_id);
                                        }
                                        
                                        // Now establish media flow to the remote endpoint
                                        // The establish_media_flow will also start audio transmission
                                        let remote_addr_str = negotiated.remote_addr.to_string();
                                        
                                        // Get dialog ID for this session
                                        let dialog_id = {
                                            let mapping = self.media_manager.session_mapping.read().await;
                                            mapping.get(&session_id).cloned()
                                        };
                                        
                                        if let Some(dialog_id) = dialog_id {
                                            if let Err(e) = self.media_manager.controller.establish_media_flow(&dialog_id, negotiated.remote_addr).await {
                                                tracing::error!("Failed to establish media flow: {}", e);
                                            } else {
                                                tracing::info!("✅ Established media flow to {} for session {}", remote_addr_str, session_id);
                                            }
                                        } else {
                                            tracing::warn!("No dialog ID found for session {} - cannot establish media flow", session_id);
                                        }
                                    }
                                }
                                Err(e) => {
                                    tracing::error!("SDP negotiation failed: {}", e);
                                }
                            }
                        }
                    } else {
                        // Media session doesn't exist yet - this happens when we receive 
                        // the remote SDP before media creation (RFC 3261 compliant flow)
                        // Store the remote SDP for later processing
                        tracing::info!("Media session not yet created for {}, storing remote SDP for later", session_id);
                        
                        // Store the remote SDP in the MediaManager's storage
                        let mut sdp_storage = self.media_manager.sdp_storage.write().await;
                        let entry = sdp_storage.entry(session_id.clone()).or_insert((None, None));
                        entry.1 = Some(sdp.clone());
                        tracing::info!("Stored remote SDP for session {} in MediaManager storage", session_id);
                        
                        // The media session will be created later when we receive the
                        // rfc_compliant_media_creation_uac event, and at that point
                        // it will pick up the stored remote SDP
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
                            
                            // Update the media session with the remote SDP (their offer)
                            // This stores the SDP and configures the remote RTP endpoint
                            if let Err(e) = self.media_manager.update_media_session(&session_id, &their_offer).await {
                                tracing::error!("Failed to update media session with remote SDP: {}", e);
                            } else {
                                tracing::info!("Updated media session with remote SDP (offer) for session {}", session_id);
                                
                                // Store the negotiated SDP in the registry
                                if let Err(e) = self.registry.update_session_sdp(&session_id, Some(our_answer.clone()), Some(their_offer.clone())).await {
                                    tracing::error!("Failed to store negotiated SDP in registry: {}", e);
                                } else {
                                    tracing::info!("Stored negotiated SDP in registry for session {}", session_id);
                                }
                            }
                            
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