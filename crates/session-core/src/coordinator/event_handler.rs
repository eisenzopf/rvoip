//! Event handling implementation for SessionCoordinator

use std::sync::Arc;
use tokio::sync::mpsc;
use crate::api::types::{SessionId, CallState, IncomingCall, CallDecision};
use crate::errors::{Result, SessionError};
use crate::manager::events::SessionEvent;
use crate::session::Session;
use super::SessionCoordinator;

impl SessionCoordinator {
    /// Main event loop that handles all session events using broadcast channel
    pub(crate) async fn run_event_loop(self: Arc<Self>) {
        tracing::info!("Starting main coordinator event loop (unified broadcast)");

        // Subscribe to the unified broadcast channel
        match self.event_processor.subscribe().await {
            Ok(mut subscriber) => {
                while let Ok(event) = subscriber.receive().await {
                    // Non-blocking: spawn event handling to avoid deadlocks
                    // The state machines should handle out-of-order events
                    let self_clone = self.clone();
                    tokio::spawn(async move {
                        if let Err(e) = self_clone.handle_event(event).await {
                            tracing::error!("Error handling event: {}", e);
                        }
                    });
                }
            }
            Err(e) => {
                tracing::error!("Failed to subscribe to event processor: {}", e);
            }
        }

        tracing::info!("Main coordinator event loop ended");
    }

    /// Handle a session event
    async fn handle_event(self: &Arc<Self>, event: SessionEvent) -> Result<()> {
        tracing::debug!("🎯 COORDINATOR: Handling event: {:?}", event);
        tracing::debug!("Handling event: {:?}", event);

        // Event is already published through the broadcast channel
        // since this handler is now a subscriber to that channel
        // No need to re-publish here

        match event {
            SessionEvent::SessionCreated { session_id, from, to, call_state } => {
                self.handle_session_created(session_id, from, to, call_state).await?;
            }
            
            SessionEvent::IncomingCall { session_id, dialog_id, from, to, sdp, headers } => {
                // Handle incoming call forwarded from dialog coordinator
                self.handle_incoming_call(session_id, dialog_id, from, to, sdp, headers).await?;
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
                tracing::debug!("🎯 COORDINATOR: Matched SessionTerminating event (Phase 1) for {} - {}", session_id, reason);
                self.handle_session_terminating(session_id, reason).await?;
            }
            
            SessionEvent::SessionTerminated { session_id, reason } => {
                tracing::debug!("🎯 COORDINATOR: Matched SessionTerminated event (Phase 2) for {} - {}", session_id, reason);
                self.handle_session_terminated(session_id, reason).await?;
            }
            
            SessionEvent::CleanupConfirmation { session_id, layer } => {
                tracing::debug!("🧹 COORDINATOR: Cleanup confirmation from {} for session {}", layer, session_id);
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
            
            SessionEvent::IncomingTransferRequest { session_id, target_uri, referred_by, .. } => {
                // Notify handler about incoming transfer request
                if let Some(handler) = &self.handler {
                    let accept = handler.on_incoming_transfer_request(
                        &session_id, 
                        &target_uri, 
                        referred_by.as_deref()
                    ).await;
                    
                    if !accept {
                        // Handler rejected the transfer
                        tracing::info!("Handler rejected transfer request for session {}", session_id);
                        // TODO: Send 603 Decline response through dialog layer
                    } else {
                        tracing::info!("Handler accepted transfer request for session {}", session_id);
                        // The transfer will proceed as normal
                    }
                }
            }
            
            SessionEvent::TransferProgress { session_id, status } => {
                // Notify handler about transfer progress
                if let Some(handler) = &self.handler {
                    handler.on_transfer_progress(&session_id, &status).await;
                }
            }
            
            // Shutdown events - orchestrate proper shutdown sequence
            SessionEvent::ShutdownInitiated { reason } => {
                self.handle_shutdown_initiated(reason).await?;
            }
            SessionEvent::ShutdownReady { component } => {
                self.handle_shutdown_ready(component).await?;
            }
            SessionEvent::ShutdownNow { component } => {
                self.handle_shutdown_now(component).await?;
            }
            SessionEvent::ShutdownComplete { component } => {
                self.handle_shutdown_complete(component).await?;
            }
            SessionEvent::SystemShutdownComplete => {
                tracing::info!("System shutdown complete");
            }
            
            _ => {
                tracing::debug!("Unhandled event type");
            }
        }

        Ok(())
    }

    /// Handle session created event
    async fn handle_session_created(
        self: &Arc<Self>,
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
                // Spawn media session creation in background
                let self_clone = self.clone();
                let session_id_clone = session_id.clone();
                tokio::spawn(async move {
                    if let Err(e) = self_clone.start_media_session(&session_id_clone).await {
                        tracing::error!("Failed to start media session for {}: {}", session_id_clone, e);
                    }
                });
            }
            _ => {}
        }

        Ok(())
    }

    /// Handle session state change
    pub(crate) async fn handle_state_changed(
        self: &Arc<Self>,
        session_id: SessionId,
        old_state: CallState,
        new_state: CallState,
    ) -> Result<()> {
        tracing::debug!("🔄 handle_state_changed called: {} {:?} -> {:?}", session_id, old_state, new_state);

        match (old_state, new_state.clone()) {
            // Call becomes active
            (CallState::Ringing, CallState::Active) |
            (CallState::Initiating, CallState::Active) => {
                tracing::debug!("📞 Starting media session for newly active call: {}", session_id);
                
                // Spawn media session creation in background to avoid blocking event processing
                let self_clone = self.clone();
                let session_id_clone = session_id.clone();
                tokio::spawn(async move {
                    if let Err(e) = self_clone.start_media_session(&session_id_clone).await {
                        tracing::error!("Failed to start media session for {}: {}", session_id_clone, e);
                    }
                });
                
                // Notify handler that call is established (also in background)
                if let Some(handler) = &self.handler {
                    let handler_clone = handler.clone();
                    let registry_clone = self.registry.clone();
                    let media_manager_clone = self.media_manager.clone();
                    let session_id_clone = session_id.clone();
                    
                    tokio::spawn(async move {
                        if let Ok(Some(session)) = registry_clone.get_session(&session_id_clone).await {
                            // Get SDP information if available
                            let media_info = media_manager_clone.get_media_info(&session_id_clone).await.ok().flatten();
                            
                            tracing::info!("Media info available: {}", media_info.is_some());
                            if let Some(ref info) = media_info {
                                tracing::info!("Local SDP length: {:?}", info.local_sdp.as_ref().map(|s| s.len()));
                                tracing::info!("Remote SDP length: {:?}", info.remote_sdp.as_ref().map(|s| s.len()));
                            }
                            
                            let local_sdp = media_info.as_ref().and_then(|m| m.local_sdp.clone());
                            let remote_sdp = media_info.as_ref().and_then(|m| m.remote_sdp.clone());
                            
                            // Store SDP in the registry so hold/resume can access it
                            if local_sdp.is_some() || remote_sdp.is_some() {
                                if let Err(e) = registry_clone.update_session_sdp(&session_id_clone, local_sdp.clone(), remote_sdp.clone()).await {
                                    tracing::error!("Failed to store SDP in registry: {}", e);
                                } else {
                                    tracing::info!("Stored SDP in registry for session {}", session_id_clone);
                                }
                            }
                            
                            tracing::info!("Notifying handler about call {} establishment", session_id_clone);
                            handler_clone.on_call_established(session.as_call_session().clone(), local_sdp, remote_sdp).await;
                        }
                    });
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
        tracing::debug!("🟡 COORDINATOR: handle_session_terminating called for session {} (Phase 1) with reason: {}", session_id, reason);
        tracing::info!("Session {} terminating (Phase 1): {}", session_id, reason);

        // Update session state to Terminating
        if let Ok(Some(session)) = self.registry.get_session(&session_id).await {
            let old_state = session.state().clone();
            
            // Update the session state to Terminating
            if let Err(e) = self.registry.update_session_state(&session_id, CallState::Terminating).await {
                tracing::error!("Failed to update session to Terminating state: {}", e);
            } else {
                // Emit state change event
                let _ = self.publish_event(SessionEvent::StateChanged {
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
        tracing::debug!("🧹 COORDINATOR: handle_cleanup_confirmation called for session {} from layer {}", session_id, layer);
        tracing::info!("Cleanup confirmation from {} for session {}", layer, session_id);
        
        use super::coordinator::CleanupLayer;
        use std::time::Duration;
        
        let mut pending_cleanups = self.pending_cleanups.lock().await;
        
        if let Some(tracker) = pending_cleanups.get_mut(&session_id) {
            // Mark the appropriate layer as done
            match layer.as_str() {
                "Media" => {
                    tracker.media_done = true;
                    tracing::debug!("✓ Media cleanup complete for session {}", session_id);
                }
                "Client" => {
                    tracker.client_done = true;
                    tracing::debug!("✓ Client cleanup complete for session {}", session_id);
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
                tracing::debug!("🔴 Triggering Phase 2 termination for session {}", session_id);
                let _ = self.publish_event(SessionEvent::SessionTerminated {
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
        tracing::debug!("🔴 COORDINATOR: handle_session_terminated called for session {} with reason: {}", session_id, reason);
        tracing::info!("Session {} terminated: {}", session_id, reason);

        // Stop media
        self.stop_media_session(&session_id).await?;
        
        // Clean up From URI mappings for this session
        self.dialog_coordinator.untrack_from_uri_for_session(&session_id);

        // Update session state to Terminated before notifying handler
        let mut call_session_for_handler = None;
        if let Ok(Some(session)) = self.registry.get_session(&session_id).await {
            let old_state = session.state().clone();
            
            // Update the session state in registry FIRST
            if let Err(e) = self.registry.update_session_state(&session_id, CallState::Terminated).await {
                tracing::error!("Failed to update session to Terminated state: {}", e);
            } else {
                // Emit state change event
                let _ = self.publish_event(SessionEvent::StateChanged {
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
            tracing::debug!("🔔 COORDINATOR: Handler exists, checking for session {}", session_id);
            if let Some(call_session) = call_session_for_handler {
                tracing::debug!("✅ COORDINATOR: Found session {}, calling handler.on_call_ended", session_id);
                tracing::info!("Notifying handler about session {} termination", session_id);
                handler.on_call_ended(call_session, &reason).await;
            } else {
                tracing::debug!("❌ COORDINATOR: Session {} not found in registry", session_id);
            }
        } else {
            tracing::debug!("⚠️ COORDINATOR: No handler configured");
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
                            // Handle state change directly instead of sending another event
                            // This avoids potential deadlock when processing multiple concurrent events
                            let new_state = CallState::Active;
                            
                            // First publish to subscribers
                            tracing::debug!("📢 Publishing StateChanged event: {} -> {}", old_state, new_state);
                            let publish_result = self.event_processor.publish_event(SessionEvent::StateChanged {
                                session_id: session_id.clone(),
                                old_state: old_state.clone(),
                                new_state: new_state.clone(),
                            }).await;
                            
                            if let Err(e) = publish_result {
                                tracing::error!("Failed to publish StateChanged event: {:?}", e);
                            } else {
                                tracing::debug!("✅ Successfully published StateChanged for session {}", session_id);
                            }
                            
                            // Start media session for the newly active call
                            // Since we transitioned from Initiating/Ringing to Active
                            tracing::debug!("📞 Starting media session for newly active call: {}", session_id);
                            
                            // Start media session directly (already non-blocking internally)
                            if let Err(e) = self.start_media_session(&session_id).await {
                                tracing::error!("Failed to start media session for {}: {}", session_id, e);
                            }
                            
                            // Notify handler that call is established (important for incoming calls)
                            if let Some(handler) = &self.handler {
                                let handler_clone = handler.clone();
                                let registry_clone = self.registry.clone();
                                let media_manager_clone = self.media_manager.clone();
                                let session_id_clone = session_id.clone();
                                
                                tokio::spawn(async move {
                                    if let Ok(Some(session)) = registry_clone.get_session(&session_id_clone).await {
                                        // Get SDP information if available
                                        let media_info = media_manager_clone.get_media_info(&session_id_clone).await.ok().flatten();
                                        let local_sdp = media_info.as_ref().and_then(|m| m.local_sdp.clone());
                                        let remote_sdp = media_info.as_ref().and_then(|m| m.remote_sdp.clone());
                                        
                                        tracing::info!("Notifying handler about call {} establishment (from media event)", session_id_clone);
                                        handler_clone.on_call_established(session.as_call_session().clone(), local_sdp, remote_sdp).await;
                                    }
                                });
                            }
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
                            let _ = self.publish_event(SessionEvent::SdpEvent {
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
    
    /// Handle incoming call event forwarded from dialog coordinator
    async fn handle_incoming_call(
        &self,
        session_id: SessionId,
        dialog_id: rvoip_dialog_core::DialogId,
        from: String,
        to: String,
        sdp: Option<String>,
        headers: std::collections::HashMap<String, String>,
    ) -> Result<()> {
        tracing::info!("🎯 COORDINATOR: Handling IncomingCall for session {} from dialog {}", session_id, dialog_id);
        
        // Create the session
        let mut session = Session::new(session_id.clone());
        session.call_session.from = from.clone();
        session.call_session.to = to.clone();
        session.call_session.state = CallState::Initiating;
        
        // Register the session
        self.registry.register_session(session).await?;
        
        // Send SessionCreated event
        self.publish_event(SessionEvent::SessionCreated {
            session_id: session_id.clone(),
            from: from.clone(),
            to: to.clone(),
            call_state: CallState::Initiating,
        }).await?;
        
        // Call the handler to decide whether to accept or reject
        if let Some(handler) = &self.handler {
            let incoming_call = IncomingCall {
                id: session_id.clone(),
                from: from.clone(),
                to: to.clone(),
                sdp,
                headers,
                received_at: std::time::Instant::now(),
            };
            
            let decision = handler.on_incoming_call(incoming_call).await;
            tracing::info!("Handler decision for session {}: {:?}", session_id, decision);
            
            // Process the decision through the dialog coordinator
            match decision {
                CallDecision::Accept(sdp_answer) => {
                    // Accept the call through dialog manager
                    if let Err(e) = self.dialog_manager.accept_incoming_call(&session_id, sdp_answer).await {
                        tracing::error!("Failed to accept incoming call {}: {}", session_id, e);
                    }
                }
                CallDecision::Reject(reason) => {
                    // Reject the call through dialog manager
                    // For now, just terminate the session
                    if let Err(e) = self.dialog_manager.terminate_session(&session_id).await {
                        tracing::error!("Failed to reject incoming call {}: {}", session_id, e);
                    }
                }
                CallDecision::Defer => {
                    // The handler will decide later
                    tracing::info!("Call decision deferred for session {}", session_id);
                }
                CallDecision::Forward(target) => {
                    // Forward/transfer the call to another destination
                    tracing::info!("Call forwarded to {} for session {}", target, session_id);
                    // For now, just reject the original call
                    if let Err(e) = self.dialog_manager.terminate_session(&session_id).await {
                        tracing::error!("Failed to forward call {}: {}", session_id, e);
                    }
                }
            }
        } else {
            tracing::warn!("No handler configured for incoming call");
            // Auto-reject if no handler
            if let Err(e) = self.dialog_manager.terminate_session(&session_id).await {
                tracing::error!("Failed to auto-reject incoming call {}: {}", session_id, e);
            }
        }
        
        Ok(())
    }
    
    // ========== SHUTDOWN EVENT HANDLERS ==========
    
    /// Handle shutdown initiated event - start the shutdown sequence
    async fn handle_shutdown_initiated(&self, reason: Option<String>) -> Result<()> {
        tracing::info!("🛑 Shutdown initiated: {:?}", reason);
        tracing::debug!("📤 SHUTDOWN: Broadcasting shutdown request to all components");
        
        // First, tell all components to prepare for shutdown
        // They should stop accepting new work but continue processing existing work
        
        // Start with bottom layer - Transport
        self.publish_event(SessionEvent::ShutdownNow {
            component: "UdpTransport".to_string(),
        }).await?;
        
        Ok(())
    }
    
    /// Handle component ready for shutdown
    async fn handle_shutdown_ready(&self, component: String) -> Result<()> {
        tracing::info!("Component {} is ready for shutdown", component);
        tracing::debug!("📥 SHUTDOWN: {} is ready for shutdown", component);
        
        // Components report ready when they've stopped accepting new work
        // We can proceed with shutting them down
        
        Ok(())
    }
    
    /// Handle shutdown now for a specific component
    async fn handle_shutdown_now(&self, component: String) -> Result<()> {
        tracing::info!("Shutting down component: {}", component);
        tracing::debug!("🔻 SHUTDOWN: Shutting down {} now", component);
        
        match component.as_str() {
            "UdpTransport" => {
                // Transport doesn't have direct access, it will be stopped via TransactionManager
                // Just emit completion for now
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                self.publish_event(SessionEvent::ShutdownComplete {
                    component: "UdpTransport".to_string(),
                }).await?;
            }
            "TransactionManager" => {
                // Transaction manager shutdown is triggered via dialog manager
                // Just emit completion for now
                tokio::time::sleep(std::time::Duration::from_millis(50)).await;
                self.publish_event(SessionEvent::ShutdownComplete {
                    component: "TransactionManager".to_string(),
                }).await?;
            }
            "DialogManager" => {
                // Actually stop the dialog manager
                if let Err(e) = self.dialog_manager.stop().await {
                    tracing::warn!("Error stopping dialog manager: {}", e);
                }
                // Emit completion
                self.publish_event(SessionEvent::ShutdownComplete {
                    component: "DialogManager".to_string(),
                }).await?;
            }
            _ => {
                tracing::warn!("Unknown component for shutdown: {}", component);
            }
        }
        
        Ok(())
    }
    
    /// Handle component shutdown complete
    async fn handle_shutdown_complete(&self, component: String) -> Result<()> {
        tracing::info!("Component {} has completed shutdown", component);
        tracing::debug!("✅ SHUTDOWN: {} has completed shutdown", component);
        
        // When a component completes, trigger the next one in sequence
        match component.as_str() {
            "UdpTransport" => {
                // Transport done, now shutdown transaction manager
                tracing::debug!("📤 SHUTDOWN: Transport done, shutting down TransactionManager");
                self.publish_event(SessionEvent::ShutdownNow {
                    component: "TransactionManager".to_string(),
                }).await?;
            }
            "TransactionManager" => {
                // Transaction done, now shutdown dialog manager
                tracing::debug!("📤 SHUTDOWN: TransactionManager done, shutting down DialogManager");
                self.publish_event(SessionEvent::ShutdownNow {
                    component: "DialogManager".to_string(),
                }).await?;
            }
            "DialogManager" => {
                // All components done, signal system shutdown complete
                tracing::debug!("📤 SHUTDOWN: All components done, system shutdown complete");
                self.publish_event(SessionEvent::SystemShutdownComplete).await?;
            }
            _ => {}
        }
        
        Ok(())
    }
} 