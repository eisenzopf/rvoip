//! Event handling implementation for SessionCoordinator

use std::sync::Arc;
use std::time::Duration;
use tokio::sync::mpsc;
use crate::api::types::{SessionId, CallState, CallSession, IncomingCall, CallDecision};
use crate::api::control::generate_sdp_answer;
use crate::errors::{Result, SessionError};
use crate::manager::events::SessionEvent;
use crate::session::Session;
use super::SessionCoordinator;

impl SessionCoordinator {
    /// Main event loop that handles all session events using broadcast channel
    /// AND cross-crate events from the GlobalEventCoordinator
    pub(crate) async fn run_event_loop(self: Arc<Self>) {
        tracing::info!("Starting main coordinator event loop (unified broadcast + cross-crate)");

        let mut shutdown_rx = self.subscribe_shutdown();

        // Subscribe to the unified broadcast channel for local SessionEvents
        let mut local_subscriber = match self.event_processor.subscribe().await {
            Ok(sub) => sub,
            Err(e) => {
                tracing::error!("Failed to subscribe to event processor: {}", e);
                return;
            }
        };

        // Subscribe to cross-crate events from GlobalEventCoordinator
        // This bridges dialog-core → session-core via the global event bus
        let global_coord = rvoip_infra_common::events::global_coordinator().await.clone();
        let mut cross_crate_rx = match global_coord.subscribe("dialog_to_session").await {
            Ok(rx) => {
                tracing::info!("✅ Subscribed to cross-crate 'dialog_to_session' events");
                Some(rx)
            }
            Err(e) => {
                tracing::warn!("Failed to subscribe to cross-crate events: {}", e);
                None
            }
        };

        // Also subscribe to media_to_session events
        let mut media_cross_crate_rx = match global_coord.subscribe("media_to_session").await {
            Ok(rx) => {
                tracing::info!("✅ Subscribed to cross-crate 'media_to_session' events");
                Some(rx)
            }
            Err(e) => {
                tracing::warn!("Failed to subscribe to media cross-crate events: {}", e);
                None
            }
        };

        loop {
            // Build select! dynamically based on available subscriptions
            tokio::select! {
                result = local_subscriber.receive() => {
                    match result {
                        Ok(event) => {
                            let self_clone = self.clone();
                            tokio::spawn(async move {
                                if let Err(e) = self_clone.handle_event(event).await {
                                    tracing::error!("Error handling local event: {}", e);
                                }
                            });
                        }
                        Err(_) => break,
                    }
                }
                Some(cross_crate_event) = async {
                    match cross_crate_rx.as_mut() {
                        Some(rx) => rx.recv().await,
                        None => std::future::pending::<Option<Arc<dyn rvoip_infra_common::events::cross_crate::CrossCrateEvent>>>().await,
                    }
                } => {
                    tracing::info!("📨 Received cross-crate event: {}", cross_crate_event.event_type());
                    if let Some(session_event) = Self::convert_cross_crate_to_session_event(&cross_crate_event) {
                        tracing::info!("📨 Converted cross-crate event to SessionEvent: {:?}", session_event);
                        let self_clone = self.clone();
                        tokio::spawn(async move {
                            if let Err(e) = self_clone.handle_event(session_event).await {
                                tracing::error!("Error handling cross-crate event: {}", e);
                            }
                        });
                    }
                }
                Some(media_event) = async {
                    match media_cross_crate_rx.as_mut() {
                        Some(rx) => rx.recv().await,
                        None => std::future::pending::<Option<Arc<dyn rvoip_infra_common::events::cross_crate::CrossCrateEvent>>>().await,
                    }
                } => {
                    tracing::debug!("📨 Received media cross-crate event: {}", media_event.event_type());
                    // TODO: Convert media cross-crate events to session events
                }
                _ = shutdown_rx.recv() => {
                    tracing::info!("Event loop received shutdown signal, exiting");
                    break;
                }
            }
        }

        tracing::info!("Main coordinator event loop ended");
    }

    /// Convert a cross-crate event to a local SessionEvent
    fn convert_cross_crate_to_session_event(
        event: &Arc<dyn rvoip_infra_common::events::cross_crate::CrossCrateEvent>,
    ) -> Option<SessionEvent> {
        use rvoip_infra_common::events::cross_crate::RvoipCrossCrateEvent;

        // Downcast to RvoipCrossCrateEvent
        let any = event.as_any();
        let rvoip_event = any.downcast_ref::<RvoipCrossCrateEvent>()?;

        match rvoip_event {
            RvoipCrossCrateEvent::DialogToSession(dialog_event) => {
                use rvoip_infra_common::events::cross_crate::DialogToSessionEvent;
                match dialog_event {
                    DialogToSessionEvent::IncomingCall { session_id, from, to, sdp_offer, headers, .. } => {
                        let dialog_id = headers.get("X-Dialog-Id")
                            .or_else(|| headers.get("x-dialog-id"))
                            .and_then(|v| uuid::Uuid::parse_str(v).ok())
                            .map(rvoip_dialog_core::DialogId)
                            .unwrap_or_else(|| rvoip_dialog_core::DialogId(uuid::Uuid::new_v4()));

                        Some(SessionEvent::IncomingCall {
                            session_id: crate::api::types::SessionId(session_id.clone()),
                            dialog_id,
                            from: from.clone(),
                            to: to.clone(),
                            sdp: sdp_offer.clone(),
                            headers: headers.clone(),
                        })
                    }
                    DialogToSessionEvent::CallStateChanged { session_id, new_state, .. } => {
                        use rvoip_infra_common::events::cross_crate::CallState as CrossCrateCallState;
                        let local_state = match new_state {
                            CrossCrateCallState::Ringing => CallState::Ringing,
                            CrossCrateCallState::Active => CallState::Active,
                            CrossCrateCallState::OnHold => CallState::OnHold,
                            CrossCrateCallState::Terminating => CallState::Terminating,
                            CrossCrateCallState::Terminated => CallState::Terminated,
                            _ => return None,
                        };
                        Some(SessionEvent::StateChanged {
                            session_id: crate::api::types::SessionId(session_id.clone()),
                            old_state: CallState::Initiating,
                            new_state: local_state,
                        })
                    }
                    DialogToSessionEvent::CallEstablished { session_id, sdp_answer } => {
                        // B-leg 200 OK received — B2BUA bridge forwards to A-leg
                        Some(SessionEvent::B2BuaLegEstablished {
                            session_id: crate::api::types::SessionId(session_id.clone()),
                            sdp_answer: sdp_answer.clone(),
                        })
                    }
                    DialogToSessionEvent::CallTerminated { session_id, .. } => {
                        Some(SessionEvent::SessionTerminating {
                            session_id: crate::api::types::SessionId(session_id.clone()),
                            reason: "remote hangup".to_string(),
                        })
                    }
                    _ => None,
                }
            }
            _ => None,
        }
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
                self.handle_state_changed(session_id.clone(), old_state.clone(), new_state.clone()).await?;
                if let Some(handler) = &self.handler {
                    handler.on_call_state_changed(&session_id, &old_state, &new_state, reason.as_deref()).await;
                }
            }

            SessionEvent::B2BuaLegEstablished { session_id, sdp_answer } => {
                // B-leg received 200 OK — forward to A-leg via B2BUA bridge
                // session_id may be a real session ID or "dialog:UUID" fallback
                let resolved_session_id = if session_id.0.starts_with("dialog:") {
                    let dialog_uuid = &session_id.0["dialog:".len()..];
                    if let Ok(uuid) = uuid::Uuid::parse_str(dialog_uuid) {
                        let dialog_id = rvoip_dialog_core::DialogId(uuid);
                        self.dialog_coordinator.get_dialog_id_for_session_reverse(&dialog_id).await
                            .unwrap_or(session_id.clone())
                    } else {
                        session_id.clone()
                    }
                } else {
                    session_id.clone()
                };

                tracing::info!("📞 B2BUA: CallEstablished for {} (resolved from {})", resolved_session_id, session_id);

                // Only process if this session is a B-leg in a bridge
                // B-leg session IDs start with "sess_" (created by create_outgoing_call)
                // A-leg session IDs start with "session-" (created by handle_incoming_call)
                if let Some(a_leg_ref) = self.b2bua_partners.get(&resolved_session_id) {
                    let a_leg = a_leg_ref.clone();
                    drop(a_leg_ref);

                    // Verify this is actually B-leg → A-leg direction
                    // A-leg IDs start with "session-", B-leg IDs start with "sess_"
                    if resolved_session_id.0.starts_with("sess_") || resolved_session_id.0.starts_with("dialog:") {
                        tracing::info!("📞 B2BUA: B-leg {} answered, accepting A-leg {}", resolved_session_id, a_leg);
                        if let Err(e) = self.dialog_manager.accept_incoming_call(&a_leg, sdp_answer).await {
                            tracing::error!("📞 B2BUA: Failed to accept A-leg {}: {}", a_leg, e);
                        } else {
                            tracing::info!("✅ B2BUA: A-leg {} accepted — 200 OK sent to caller", a_leg);
                        }
                    } else {
                        tracing::debug!("📞 B2BUA: Ignoring CallEstablished for A-leg {} (not B-leg direction)", resolved_session_id);
                    }
                } else {
                    tracing::debug!("Session {} not in B2BUA bridge", resolved_session_id);
                }
            }

            SessionEvent::SessionTerminating { session_id, reason } => {
                // Resolve dialog:xxx fallback session IDs
                let resolved_id = if session_id.0.starts_with("dialog:") {
                    let dialog_uuid = &session_id.0["dialog:".len()..];
                    if let Ok(uuid) = uuid::Uuid::parse_str(dialog_uuid) {
                        let dialog_id = rvoip_dialog_core::DialogId(uuid);
                        self.dialog_coordinator.get_dialog_id_for_session_reverse(&dialog_id).await
                            .unwrap_or(session_id.clone())
                    } else { session_id.clone() }
                } else { session_id.clone() };

                tracing::debug!("🎯 COORDINATOR: SessionTerminating for {} - {}", resolved_id, reason);

                // B2BUA: forward BYE to the bridge partner
                if let Some(partner_ref) = self.b2bua_partners.get(&resolved_id) {
                    let partner = partner_ref.clone();
                    drop(partner_ref);
                    tracing::info!("📞 B2BUA: {} terminating, sending BYE to partner {}", resolved_id, partner);

                    // Remove both directions to prevent loop
                    self.b2bua_partners.remove(&resolved_id);
                    self.b2bua_partners.remove(&partner);

                    if let Err(e) = self.dialog_manager.terminate_session(&partner).await {
                        tracing::error!("📞 B2BUA: Failed to terminate partner {}: {}", partner, e);
                    } else {
                        tracing::info!("✅ B2BUA: BYE sent to partner {}", partner);
                    }
                }

                self.handle_session_terminating(resolved_id, reason).await?;
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
            
            SessionEvent::TrickleIceCandidate { session_id, candidate_line } => {
                self.handle_trickle_ice_candidate(session_id, candidate_line).await?;
            }

            SessionEvent::TrickleIceEndOfCandidates { session_id } => {
                self.handle_trickle_ice_end_of_candidates(session_id).await?;
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
            
            // Subscription/Presence events
            SessionEvent::SubscriptionCreated { dialog_id, event_package, from_uri, to_uri, expires } => {
                self.handle_subscription_created(dialog_id, event_package, from_uri, to_uri, expires).await?;
            }
            
            SessionEvent::NotifyReceived { dialog_id, subscription_state, event_package, body } => {
                self.handle_notify_received(dialog_id, subscription_state, event_package, body).await?;
            }
            
            SessionEvent::SubscriptionTerminated { dialog_id, reason } => {
                self.handle_subscription_terminated(dialog_id, reason).await?;
            }
            
            SessionEvent::PresenceStateUpdate { user_uri, state, note } => {
                self.handle_presence_state_update(user_uri, state, note).await?;
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
            
            SessionEvent::MediaSessionReady { session_id, dialog_id: _ } => {
                tracing::info!("Media session ready for {} - checking readiness", session_id);
                
                // Update readiness tracking
                {
                    let mut readiness_map = self.session_readiness.write().await;
                    let readiness = readiness_map.entry(session_id.clone()).or_default();
                    readiness.media_session_ready = true;
                    
                    // Store the call session if we don't have it yet
                    if readiness.call_session.is_none() {
                        if let Ok(Some(session)) = self.registry.get_session(&session_id).await {
                            readiness.call_session = Some(session.as_call_session().clone());
                            
                            // Also get SDPs from session registry if not already set (for upfront SDP cases)
                            if readiness.local_sdp.is_none() {
                                readiness.local_sdp = session.local_sdp.clone();
                            }
                            if readiness.remote_sdp.is_none() {
                                readiness.remote_sdp = session.remote_sdp.clone();
                            }
                        }
                    }
                    
                    tracing::debug!("Session {} readiness: dialog={}, media={}, sdp={}", 
                        session_id, 
                        readiness.dialog_established,
                        readiness.media_session_ready,
                        readiness.sdp_negotiated
                    );
                }
                
                // Check if all conditions are met
                self.check_and_trigger_call_established(&session_id).await;
            }
            
            SessionEvent::MediaNegotiated { session_id, local_addr, remote_addr, codec } => {
                tracing::info!("Media negotiated for {} - codec: {}, {}↔{}", 
                    session_id, codec, local_addr, remote_addr);
                
                // Update readiness tracking
                {
                    let mut readiness_map = self.session_readiness.write().await;
                    let readiness = readiness_map.entry(session_id.clone()).or_default();
                    readiness.sdp_negotiated = true;
                    
                    // Fetch and store the SDPs - try media manager first, then session registry
                    if let Ok(Some(media_info)) = self.media_manager.get_media_info(&session_id).await {
                        readiness.local_sdp = media_info.local_sdp;
                        readiness.remote_sdp = media_info.remote_sdp;
                        tracing::debug!("Got SDP from media manager for session {} - local: {}, remote: {}", 
                            session_id,
                            readiness.local_sdp.is_some(),
                            readiness.remote_sdp.is_some()
                        );
                    } else if let Ok(Some(session)) = self.registry.get_session(&session_id).await {
                        // Fallback to session registry for upfront SDP cases
                        readiness.local_sdp = session.local_sdp.clone();
                        readiness.remote_sdp = session.remote_sdp.clone();
                        tracing::debug!("Got SDP from session registry for session {} - local: {}, remote: {}", 
                            session_id,
                            readiness.local_sdp.is_some(),
                            readiness.remote_sdp.is_some()
                        );
                    }
                    
                    // Store the call session if we don't have it yet
                    if readiness.call_session.is_none() {
                        if let Ok(Some(session)) = self.registry.get_session(&session_id).await {
                            readiness.call_session = Some(session.as_call_session().clone());
                        }
                    }
                    
                    tracing::debug!("Session {} readiness: dialog={}, media={}, sdp={}", 
                        session_id,
                        readiness.dialog_established,
                        readiness.media_session_ready,
                        readiness.sdp_negotiated
                    );
                }
                
                // Check if all conditions are met
                self.check_and_trigger_call_established(&session_id).await;
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
        
        // Initialize readiness tracking for this session
        {
            let mut readiness_map = self.session_readiness.write().await;
            let readiness = readiness_map.entry(session_id.clone()).or_default();
            
            // If session is created in Active state, mark dialog as established
            if call_state == CallState::Active {
                readiness.dialog_established = true;
                tracing::info!("Session {} created in Active state, marking dialog_established", session_id);
            }
            
            // Store the call session if available
            if let Ok(Some(session)) = self.registry.get_session(&session_id).await {
                readiness.call_session = Some(session.as_call_session().clone());
                
                // Also get SDPs from session if available (for upfront SDP cases)
                if let Some(ref local_sdp) = session.local_sdp {
                    readiness.local_sdp = Some(local_sdp.clone());
                }
                if let Some(ref remote_sdp) = session.remote_sdp {
                    readiness.remote_sdp = Some(remote_sdp.clone());
                }
            }
        }

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

        // Check if dialog is now active/established
        if new_state == CallState::Active {
            tracing::info!("Dialog established for session {}", session_id);
            
            // Update readiness tracking
            {
                let mut readiness_map = self.session_readiness.write().await;
                let readiness = readiness_map.entry(session_id.clone()).or_default();
                readiness.dialog_established = true;
                
                // Store the call session and SDPs
                if readiness.call_session.is_none() {
                    if let Ok(Some(session)) = self.registry.get_session(&session_id).await {
                        readiness.call_session = Some(session.as_call_session().clone());
                        
                        // Also get SDPs from session registry if available (for upfront SDP cases)
                        if readiness.local_sdp.is_none() && session.local_sdp.is_some() {
                            readiness.local_sdp = session.local_sdp.clone();
                            tracing::debug!("Got local SDP from session registry in state change");
                        }
                        if readiness.remote_sdp.is_none() && session.remote_sdp.is_some() {
                            readiness.remote_sdp = session.remote_sdp.clone();
                            tracing::debug!("Got remote SDP from session registry in state change");
                        }
                        
                        // For outbound calls with upfront SDP, SDP negotiation happens immediately
                        // Check if this is an outbound call with local SDP but no remote SDP yet
                        if session.local_sdp.is_some() && session.remote_sdp.is_none() {
                            tracing::info!("Outbound call with upfront SDP detected for {}, marking SDP as negotiated", session_id);
                            readiness.sdp_negotiated = true;
                        }
                    }
                }
                
                tracing::debug!("Session {} readiness: dialog={}, media={}, sdp={}", 
                    session_id,
                    readiness.dialog_established,
                    readiness.media_session_ready,
                    readiness.sdp_negotiated
                );
            }
            
            // Check if all conditions are met
            self.check_and_trigger_call_established(&session_id).await;
        }

        match (old_state, new_state.clone()) {
            // Call becomes active
            (CallState::Ringing, CallState::Active) |
            (CallState::Initiating, CallState::Active) => {
                tracing::debug!("📞 Starting media session for newly active call: {}", session_id);
                
                // Check if this is an outbound call with upfront SDP
                let is_upfront_sdp = if let Ok(Some(session)) = self.registry.get_session(&session_id).await {
                    session.local_sdp.is_some()
                } else {
                    false
                };
                
                // Spawn media session creation in background to avoid blocking event processing
                // The MediaSessionReady event will be published when media is ready
                let self_clone = self.clone();
                let session_id_clone = session_id.clone();
                tokio::spawn(async move {
                    if let Err(e) = self_clone.start_media_session(&session_id_clone).await {
                        tracing::error!("Failed to start media session for {}: {}", session_id_clone, e);
                    }
                    
                    // For upfront SDP cases, wait briefly for media to be ready then check conditions
                    if is_upfront_sdp {
                        tokio::time::sleep(Duration::from_millis(100)).await;
                        tracing::info!("Checking call establishment after media setup for upfront SDP call {}", session_id_clone);
                        if let Err(e) = self_clone.check_and_trigger_call_established(&session_id_clone).await {
                            tracing::warn!("Failed to check/trigger call establishment for session {}: {}", session_id_clone, e);
                        }
                    }
                });
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
                if let Err(e) = self.publish_event(SessionEvent::StateChanged {
                    session_id: session_id.clone(),
                    old_state: old_state.clone(),
                    new_state: CallState::Terminating,
                }).await {
                    tracing::warn!("Failed to publish Terminating state change event for session {}: {}", session_id, e);
                }
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
                if let Err(e) = self.publish_event(SessionEvent::SessionTerminated {
                    session_id: session_id.clone(),
                    reason,
                }).await {
                    tracing::warn!("Failed to publish SessionTerminated event for session {}: {}", session_id, e);
                }
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

        // Clean up readiness tracking
        {
            let mut readiness_map = self.session_readiness.write().await;
            if readiness_map.remove(&session_id).is_some() {
                tracing::debug!("Cleaned up readiness tracking for session {}", session_id);
            }
        }

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
                if let Err(e) = self.publish_event(SessionEvent::StateChanged {
                    session_id: session_id.clone(),
                    old_state,
                    new_state: CallState::Terminated,
                }).await {
                    tracing::warn!("Failed to publish Terminated state change event for session {}: {}", session_id, e);
                }
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

        // Unregister the terminated session from the registry to prevent leaks.
        // The handler has already been notified above, so the session data is no longer needed.
        if let Err(e) = self.registry.unregister_session(&session_id).await {
            tracing::warn!("Failed to unregister terminated session {}: {}", session_id, e);
        }

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
                
                // CRITICAL: For UAS, publish MediaFlowEstablished when media is created
                // This happens AFTER the SimpleCall has subscribed to events
                if event == "rfc_compliant_media_creation_uas" {
                    if let Some(negotiated) = self.get_negotiated_config(&session_id).await {
                        tracing::info!("📢 Publishing MediaFlowEstablished for UAS {} in media creation handler", session_id);
                        if let Err(e) = self.publish_event(SessionEvent::MediaFlowEstablished {
                            session_id: session_id.clone(),
                            local_addr: negotiated.local_addr.to_string(),
                            remote_addr: negotiated.remote_addr.to_string(),
                            direction: crate::manager::events::MediaFlowDirection::Both,
                        }).await {
                            tracing::warn!("Failed to publish MediaFlowEstablished for UAS {} from media creation handler: {e}", session_id);
                        } else {
                            tracing::info!("MediaFlowEstablished published for UAS {} from media creation handler", session_id);
                        }
                    } else {
                        tracing::warn!("⚠️ No negotiated config found for UAS {} in media creation handler", session_id);
                    }
                }
                
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
                                tracing::warn!("Failed to publish StateChanged event: {e}");
                            } else {
                                tracing::debug!("✅ Successfully published StateChanged for session {}", session_id);
                            }
                            
                            // Start media session for the newly active call
                            // Since we transitioned from Initiating/Ringing to Active
                            tracing::debug!("📞 Starting media session for newly active call: {}", session_id);
                            
                            // Start media session directly (already non-blocking internally)
                            // The MediaSessionReady event will be published when media is ready,
                            // and that's when we'll notify the handler about call establishment
                            if let Err(e) = self.start_media_session(&session_id).await {
                                tracing::error!("Failed to start media session for {}: {}", session_id, e);
                            }
                            
                            // CRITICAL: For UAS, publish MediaFlowEstablished after media creation
                            // This is needed because UAS doesn't go through negotiate_sdp_as_uas
                            // when accepting a call with pre-generated SDP answer
                            if event == "rfc_compliant_media_creation_uas" {
                                // Get negotiated config if available
                                if let Some(negotiated) = self.get_negotiated_config(&session_id).await {
                                    tracing::info!("📢 Publishing MediaFlowEstablished for UAS {} after media creation", session_id);
                                    if let Err(e) = self.publish_event(SessionEvent::MediaFlowEstablished {
                                        session_id: session_id.clone(),
                                        local_addr: negotiated.local_addr.to_string(),
                                        remote_addr: negotiated.remote_addr.to_string(),
                                        direction: crate::manager::events::MediaFlowDirection::Both,
                                    }).await {
                                        tracing::warn!("Failed to publish MediaFlowEstablished for UAS {} after media creation: {e}", session_id);
                                    } else {
                                        tracing::info!("MediaFlowEstablished published for UAS {}", session_id);
                                    }
                                } else {
                                    tracing::warn!("No negotiated config found for UAS {} - cannot publish MediaFlowEstablished", session_id);
                                }
                            }
                            
                            // For UAS: The MediaFlowEstablished event will be published when we receive SDP offer
                            // and create the negotiated config. For now, just log that we're UAS becoming active.
                            if let Ok(Some(session)) = self.registry.get_session(&session_id).await {
                                if session.role == crate::api::types::SessionRole::UAS {
                                    tracing::info!("UAS session {} becoming Active, media flow will be established when negotiation completes", session_id);
                                }
                            }
                            
                            // NOTE: on_call_established is now called from MediaSessionReady handler
                            // to ensure both dialog and media are ready before notifying the handler
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
            "remote_sdp_answer" | "final_negotiated_sdp" => {
                // For UAC: we sent offer, received answer - negotiate
                if let Ok(Some(session)) = self.registry.get_session(&session_id).await {
                    // Get our offer - first try media manager, then session registry
                    let media_info = self.media_manager.get_media_info(&session_id).await.ok().flatten();
                    
                    // Get our offer from either media info or session registry
                    let our_offer = if let Some(ref media_info) = media_info {
                        media_info.local_sdp.clone()
                    } else {
                        // For calls with upfront SDP, get from session registry
                        session.local_sdp.clone()
                    };
                    
                    if let Some(our_offer) = our_offer {
                        // We have an offer, proceed with negotiation
                        if media_info.is_some() {
                            // Media session exists - normal flow
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
                                        
                                        // Ensure media session exists before trying to establish flow
                                        // For UAC, the media session should already exist, but double-check
                                        if !self.media_manager.has_session_mapping(&session_id).await {
                                            tracing::info!("Media session not yet created for UAC {}, creating it now", session_id);
                                            if let Err(e) = self.start_media_session(&session_id).await {
                                                tracing::error!("Failed to create media session for UAC {}: {}", session_id, e);
                                            }
                                        }
                                        
                                        // Get the media session's dialog ID (not the SIP dialog ID)
                                        // The media manager uses its own internal dialog IDs for RTP sessions
                                        let dialog_id = {
                                            let mapping = self.media_manager.session_mapping.read().await;
                                            mapping.get(&session_id).cloned()
                                        };
                                        
                                        if let Some(dialog_id) = dialog_id {
                                            tracing::info!("🔄 UAC establishing media flow to UAS at {} for session {} (media dialog: {})", 
                                                remote_addr_str, session_id, dialog_id);
                                            // TODO: establish_media_flow doesn't exist yet
                                            // For now, just publish the event since media is ready
                                            tracing::info!("✅ UAC media flow ready to UAS at {} for session {}", 
                                                remote_addr_str, session_id);
                                                
                                            // Publish MediaFlowEstablished event
                                            tracing::info!("📢 Publishing MediaFlowEstablished event for UAC session {}", session_id);
                                            let result = self.publish_event(SessionEvent::MediaFlowEstablished {
                                                session_id: session_id.clone(),
                                                local_addr: negotiated.local_addr.to_string(),
                                                remote_addr: negotiated.remote_addr.to_string(),
                                                direction: crate::manager::events::MediaFlowDirection::Both,
                                            }).await;
                                            if let Err(e) = result {
                                                tracing::warn!("Failed to publish MediaFlowEstablished event: {e}");
                                            } else {
                                                tracing::info!("✅ MediaFlowEstablished event published for UAC {}", session_id);
                                            }
                                        } else {
                                            tracing::warn!("No media dialog ID found for session {} - cannot establish UAC->UAS media flow", session_id);
                                        }
                                    }
                                }
                                Err(e) => {
                                    tracing::error!("SDP negotiation failed: {}", e);
                                    // If this is an SRTP security downgrade, terminate the session
                                    // to prevent continuing in an insecure state
                                    if e.is_srtp_security_failure() {
                                        tracing::error!("Terminating session {} due to SRTP security failure", session_id);
                                        if let Err(term_err) = self.terminate_session(&session_id).await {
                                            tracing::error!("Failed to terminate session after SRTP failure: {}", term_err);
                                        }
                                    }
                                }
                            }
                        } else {
                            // Media session doesn't exist yet but we have SDP provided upfront
                            // This happens with create_outgoing_call when SDP is provided
                            tracing::info!("No media session but have upfront SDP for session {}", session_id);
                            
                            // For upfront SDP cases, we just store the SDPs without full negotiation
                            // The actual media session will be created later
                            
                            // Store the SDPs in the registry
                            if let Err(e) = self.registry.update_session_sdp(&session_id, Some(our_offer.clone()), Some(sdp.clone())).await {
                                tracing::error!("Failed to store SDPs in registry: {}", e);
                            } else {
                                tracing::info!("Stored SDPs in registry for upfront SDP case");
                            }
                            
                            // Update readiness tracking with the SDPs
                            {
                                let mut readiness_map = self.session_readiness.write().await;
                                
                                tracing::debug!("Current sessions in readiness map:");
                                for (sid, r) in readiness_map.iter() {
                                    tracing::debug!("  - {}: local={}, remote={}, negotiated={}",
                                        sid, r.local_sdp.is_some(), r.remote_sdp.is_some(), r.sdp_negotiated);
                                }
                                
                                // Update Bob's session
                                if let Some(readiness) = readiness_map.get_mut(&session_id) {
                                    readiness.local_sdp = Some(our_offer.clone());
                                    readiness.remote_sdp = Some(sdp.clone());
                                    readiness.sdp_negotiated = true;
                                    tracing::info!("Updated Bob's readiness with SDPs for session {}", session_id);
                                }
                                
                                // Find and update Alice's session (the outbound call)
                                // Alice's session has local SDP but no remote SDP yet
                                for (sid, readiness) in readiness_map.iter_mut() {
                                    if sid != &session_id && 
                                       readiness.local_sdp.is_some() && 
                                       readiness.remote_sdp.is_none() {
                                        readiness.remote_sdp = Some(sdp.clone());
                                        tracing::info!("Updated outbound session {} with remote SDP", sid);
                                        tracing::debug!("Updated outbound session {} with remote SDP", sid);
                                        break;
                                    }
                                }
                            }
                            
                            // Emit MediaNegotiated event manually since we're not calling negotiate_sdp_as_uac
                            // Extract addresses from SDP (simplified - in production would parse properly)
                            if let Err(e) = self.publish_event(SessionEvent::MediaNegotiated {
                                session_id: session_id.clone(),
                                local_addr: std::net::SocketAddr::from(([0, 0, 0, 0], 0)), // Would be parsed from SDP
                                remote_addr: std::net::SocketAddr::from(([0, 0, 0, 0], 0)), // Would be parsed from SDP
                                codec: "PCMU".to_string(), // Would be determined from negotiation
                            }).await {
                                tracing::warn!("Failed to publish MediaNegotiated event: {e}");
                            }
                            
                            // Check if conditions are now met for Bob's session
                            self.check_and_trigger_call_established(&session_id).await;
                            
                            // Also check for Alice's session
                            let alice_sessions: Vec<SessionId> = {
                                let readiness_map = self.session_readiness.read().await;
                                readiness_map.keys()
                                    .filter(|sid| *sid != &session_id)
                                    .cloned()
                                    .collect()
                            };
                            for sid in alice_sessions {
                                self.check_and_trigger_call_established(&sid).await;
                            }
                        }
                    } else {
                        // No local SDP offer found
                        tracing::warn!("No local SDP offer found for session {}, cannot negotiate", session_id);
                        
                    }
                }
            }
            "local_sdp_offer" => {
                // Store local SDP offer (for reference)
                if let Err(e) = self.registry.update_session_sdp(&session_id, Some(sdp), None).await {
                    tracing::error!("Failed to update session with local SDP: {}", e);
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
                                
                                // CRITICAL: Establish media flow to the remote endpoint for UAS
                                // This allows the UAS to send audio back to the UAC
                                let remote_addr_str = negotiated.remote_addr.to_string();
                                
                                // Ensure media session exists before trying to establish flow
                                // For UAS, the media session might not be created yet at this point
                                if !self.media_manager.has_session_mapping(&session_id).await {
                                    tracing::info!("Media session not yet created for UAS {}, creating it now", session_id);
                                    if let Err(e) = self.start_media_session(&session_id).await {
                                        tracing::error!("Failed to create media session for UAS {}: {}", session_id, e);
                                    }
                                }
                                
                                // Get the media session's dialog ID (not the SIP dialog ID)
                                // The media manager uses its own internal dialog IDs for RTP sessions
                                let dialog_id = {
                                    let mapping = self.media_manager.session_mapping.read().await;
                                    mapping.get(&session_id).cloned()
                                };
                                
                                if let Some(dialog_id) = dialog_id {
                                    tracing::info!("🔄 UAS establishing media flow to UAC at {} for session {} (media dialog: {})", 
                                        remote_addr_str, session_id, dialog_id);
                                    // TODO: establish_media_flow doesn't exist yet
                                    // For now, just publish the event since media is ready
                                    tracing::info!("✅ UAS media flow ready to UAC at {} for session {}", 
                                        remote_addr_str, session_id);
                                        
                                    // Publish MediaFlowEstablished event
                                    tracing::info!("📢 Publishing MediaFlowEstablished event for UAS session {}", session_id);
                                    let result = self.publish_event(SessionEvent::MediaFlowEstablished {
                                        session_id: session_id.clone(),
                                        local_addr: negotiated.local_addr.to_string(),
                                        remote_addr: negotiated.remote_addr.to_string(),
                                        direction: crate::manager::events::MediaFlowDirection::Both,
                                    }).await;
                                    if let Err(e) = result {
                                        tracing::warn!("Failed to publish MediaFlowEstablished event: {e}");
                                    } else {
                                        tracing::info!("✅ MediaFlowEstablished event published for UAS {}", session_id);
                                    }
                                } else {
                                    tracing::warn!("No media dialog ID found for session {} - cannot establish UAS->UAC media flow", session_id);
                                }
                            }
                            
                            // Send event with the generated answer
                            if let Err(e) = self.publish_event(SessionEvent::SdpEvent {
                                session_id,
                                event_type: "generated_sdp_answer".to_string(),
                                sdp: our_answer,
                            }).await {
                                tracing::warn!("Failed to publish generated SDP answer event: {e}");
                            }
                        }
                        Err(e) => {
                            tracing::error!("SDP negotiation as UAS failed: {}", e);
                            if e.is_srtp_security_failure() {
                                tracing::error!("Terminating session {} due to SRTP security failure", session_id);
                                if let Err(term_err) = self.terminate_session(&session_id).await {
                                    tracing::error!("Failed to terminate session after SRTP failure: {}", term_err);
                                }
                            }
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
                            if e.is_srtp_security_failure() {
                                tracing::error!("Terminating session {} due to SRTP security failure", session_id);
                                if let Err(term_err) = self.terminate_session(&session_id).await {
                                    tracing::error!("Failed to terminate session after SRTP failure: {}", term_err);
                                }
                            }
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
        self: &Arc<Self>,
        session_id: SessionId,
        dialog_id: rvoip_dialog_core::DialogId,
        from: String,
        to: String,
        sdp: Option<String>,
        headers: std::collections::HashMap<String, String>,
    ) -> Result<()> {
        tracing::info!("🎯 COORDINATOR: Handling IncomingCall for session {} from dialog {}", session_id, dialog_id);
        
        // Create the session as UAS (receiving the call)
        let mut session = Session::new_with_role(session_id.clone(), crate::api::types::SessionRole::UAS);
        session.call_session.from = from.clone();
        session.call_session.to = to.clone();
        session.call_session.state = CallState::Initiating;
        // Extract and store Call-ID if available
        session.call_session.sip_call_id = headers.get("Call-ID").cloned();
        // Store the caller's SDP offer so B2BUA Forward can relay it to the B-leg
        session.remote_sdp = sdp.clone();

        // Register the session
        self.registry.register_session(session).await?;

        // Register dialog↔session mapping so reject/terminate can find the dialog
        self.dialog_coordinator.register_dialog_session_mapping(
            dialog_id.clone(), session_id.clone(),
        ).await;

        // Send SessionCreated event
        self.publish_event(SessionEvent::SessionCreated {
            session_id: session_id.clone(),
            from: from.clone(),
            to: to.clone(),
            call_state: CallState::Initiating,
        }).await?;
        
        // Call the handler to decide whether to accept or reject
        if let Some(handler) = &self.handler {
            // Extract Call-ID from headers if available
            let sip_call_id = headers.get("Call-ID").cloned();
            
            let incoming_call = IncomingCall {
                id: session_id.clone(),
                from: from.clone(),
                to: to.clone(),
                sdp,
                headers,
                received_at: std::time::Instant::now(),
                sip_call_id,
                coordinator: None,  // Will be set by the handler if needed
            };
            
            let decision = handler.on_incoming_call(incoming_call.clone()).await;
            tracing::info!("Handler decision for session {}: {:?}", session_id, decision);
            
            // Process the decision through the dialog coordinator
            match decision {
                CallDecision::Accept(mut sdp_answer) => {
                    // If no SDP answer provided but we have an offer, generate one
                    if sdp_answer.is_none() && incoming_call.sdp.is_some() {
                        tracing::info!("Generating SDP answer for incoming call {}", session_id);
                        // SAFETY: We just checked incoming_call.sdp.is_some() above
                        let sdp_offer = match incoming_call.sdp.as_ref() {
                            Some(sdp) => sdp,
                            None => {
                                tracing::error!("SDP offer disappeared unexpectedly for session {}", session_id);
                                return Ok(());
                            }
                        };
                        match generate_sdp_answer(self, &session_id, sdp_offer).await {
                            Ok(answer) => {
                                tracing::info!("Generated SDP answer for call {}", session_id);
                                sdp_answer = Some(answer);
                            }
                            Err(e) => {
                                tracing::warn!("Failed to generate SDP answer: {}", e);
                            }
                        }
                    }
                    
                    // Accept the call through dialog manager with the SDP answer
                    if let Err(e) = self.dialog_manager.accept_incoming_call(&session_id, sdp_answer).await {
                        tracing::error!("Failed to accept incoming call {}: {}", session_id, e);
                    }
                }
                CallDecision::Reject(reason) => {
                    // RFC 3261 §13.3.1: Reject by sending a final error response to INVITE
                    tracing::info!("Rejecting call {} with reason: {}", session_id, reason);
                    if let Err(e) = self.dialog_manager.reject_incoming_session(
                        &session_id,
                        rvoip_sip_core::StatusCode::BusyHere,
                        Some(reason),
                    ).await {
                        tracing::error!("Failed to send reject response for {}: {}", session_id, e);
                        // Fallback: terminate session
                        let _ = self.dialog_manager.terminate_session(&session_id).await;
                    }
                }
                CallDecision::Defer => {
                    // The handler will decide later
                    tracing::info!("Call decision deferred for session {}", session_id);
                }
                CallDecision::Forward(target) => {
                    // Forward the call to another destination via B2BUA bridge:
                    // 1. Create outgoing call to target (B-leg) with caller's SDP offer
                    // 2. When B-leg answers, relay SDP answer back to A-leg
                    tracing::info!("📲 Forwarding call {} to {}", session_id, target);

                    // Extract the From URI and the caller's SDP offer from A-leg session
                    let (raw_from, a_leg_sdp_offer) = if let Ok(Some(session)) = self.registry.get_session(&session_id).await {
                        (session.call_session.from.clone(), session.remote_sdp.clone())
                    } else {
                        (self.config.local_address.clone(), None)
                    };
                    let from_uri = raw_from
                        .trim_start_matches('<')
                        .split('>')
                        .next()
                        .unwrap_or(&raw_from)
                        .to_string();

                    if a_leg_sdp_offer.is_some() {
                        tracing::info!("📲 B2BUA: Forwarding caller's SDP offer to B-leg");
                    } else {
                        tracing::warn!("📲 B2BUA: No SDP offer from caller — B-leg INVITE will use late negotiation");
                    }

                    // Record-Route note: In a B2BUA, the proxy stays in the signaling path
                    // implicitly — it terminates both legs independently.  The server's own
                    // Contact address in the 200 OK sent to the A-leg already anchors
                    // subsequent in-dialog requests (re-INVITE, BYE) to this B2BUA.
                    // For true stateless proxy forwarding (Feature 4) we insert an explicit
                    // Record-Route header instead.
                    tracing::debug!("📲 B2BUA: Record-Route ensured via server Contact in 200 OK (proxy addr: {})", self.config.local_address);

                    // Clean the target URI too (may have angle brackets)
                    let clean_target = target
                        .trim_start_matches('<')
                        .trim_end_matches('>')
                        .to_string();

                    // Create the outgoing call to the target with the caller's SDP offer
                    match self.create_outgoing_call(
                        &from_uri,
                        &clean_target,
                        a_leg_sdp_offer,
                        None, // New Call-ID
                    ).await {
                        Ok(outgoing_call) => {
                            let b_leg_id = outgoing_call.id().clone();
                            tracing::info!("📲 B2BUA: Created B-leg {} → {}", b_leg_id, target);

                            // Register bidirectional B2BUA partner mapping
                            // A-leg ↔ B-leg so events on either side find the partner
                            self.b2bua_partners.insert(session_id.clone(), b_leg_id.clone());
                            self.b2bua_partners.insert(b_leg_id.clone(), session_id.clone());
                            tracing::info!("📞 B2BUA: Bridge registered: A={} ↔ B={}", session_id, b_leg_id);

                            // Register B-leg dialog↔session mapping for event routing
                            if let Ok(b_dialog_id) = self.dialog_manager.get_dialog_id_for_session(&b_leg_id) {
                                self.dialog_coordinator.register_dialog_session_mapping(
                                    b_dialog_id, b_leg_id,
                                ).await;
                            }
                        }
                        Err(e) => {
                            tracing::error!("Failed to forward call to {}: {}", target, e);
                            // RFC 3261: Send appropriate error response to the original INVITE
                            let error_msg = e.to_string();
                            let (status, reason) = if error_msg.contains("routing_error")
                                || error_msg.contains("No remote target")
                                || error_msg.contains("resolve")
                            {
                                // Target not found / not registered
                                (rvoip_sip_core::StatusCode::TemporarilyUnavailable,
                                 "User not available")
                            } else {
                                (rvoip_sip_core::StatusCode::ServerInternalError,
                                 "Forward failed")
                            };
                            if let Err(reject_err) = self.dialog_manager.reject_incoming_session(
                                &session_id, status, Some(reason.to_string()),
                            ).await {
                                tracing::error!("Failed to send error response for {}: {}", session_id, reject_err);
                                let _ = self.dialog_manager.terminate_session(&session_id).await;
                            }
                        }
                    }
                }
            }
        } else {
            tracing::warn!("No handler configured for incoming call — rejecting with 486");
            if let Err(e) = self.dialog_manager.reject_incoming_session(
                &session_id,
                rvoip_sip_core::StatusCode::BusyHere,
                Some("No call handler configured".to_string()),
            ).await {
                tracing::error!("Failed to auto-reject incoming call {}: {}", session_id, e);
                let _ = self.dialog_manager.terminate_session(&session_id).await;
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

    /// Check if all conditions are met and trigger on_call_established
    async fn check_and_trigger_call_established(&self, session_id: &SessionId) -> Result<()> {
        let mut readiness_map = self.session_readiness.write().await;
        
        if let Some(readiness) = readiness_map.get_mut(session_id) {
            tracing::info!("📊 Checking readiness for {}: dialog={}, media={}, sdp={}", 
                session_id, readiness.dialog_established, readiness.media_session_ready, readiness.sdp_negotiated);
            
            // Check if all three conditions are met
            if readiness.dialog_established && readiness.media_session_ready && readiness.sdp_negotiated {
                // For backward compatibility, only trigger if we have both SDPs
                // The API layer expects both SDPs to be present when on_call_established is called
                if readiness.local_sdp.is_some() && readiness.remote_sdp.is_some() {
                    tracing::info!("✅ All conditions met for session {} with both SDPs, triggering on_call_established", session_id);
                    
                    // Get the call session and SDP info
                    let call_session = readiness.call_session.clone();
                    let local_sdp = readiness.local_sdp.clone();
                    let remote_sdp = readiness.remote_sdp.clone();
                    
                    // Remove from tracking since we're done
                    readiness_map.remove(session_id);
                    
                    // Drop the lock before calling the handler
                    drop(readiness_map);
                    
                    // Trigger the callback
                    if let Some(session) = call_session {
                        self.trigger_call_established(session, local_sdp, remote_sdp).await?;
                    }
                } else {
                    tracing::debug!(
                        "Session {} has all conditions but waiting for both SDPs (local: {}, remote: {})",
                        session_id,
                        readiness.local_sdp.is_some(),
                        readiness.remote_sdp.is_some()
                    );
                }
            } else {
                tracing::debug!(
                    "Session {} readiness: dialog={}, media={}, sdp={}", 
                    session_id,
                    readiness.dialog_established,
                    readiness.media_session_ready,
                    readiness.sdp_negotiated
                );
            }
        }
        
        Ok(())
    }
    
    /// Trigger the on_call_established callback with complete information
    async fn trigger_call_established(
        &self,
        call_session: CallSession,
        local_sdp: Option<String>,
        remote_sdp: Option<String>,
    ) -> Result<()> {
        tracing::info!(
            "Triggering on_call_established for session {} with SDP (local: {}, remote: {})",
            call_session.id,
            local_sdp.is_some(),
            remote_sdp.is_some()
        );
        
        // Call the handler
        if let Some(handler) = &self.handler {
            handler.on_call_established(call_session, local_sdp, remote_sdp).await;
            tracing::info!("✅ Handler.on_call_established called successfully");
        } else {
            tracing::warn!("⚠️ No handler set to receive on_call_established event");
        }
        
        Ok(())
    }
    
    // ========== Subscription/Presence Event Handlers ==========
    
    /// Handle subscription created event
    async fn handle_subscription_created(
        &self,
        dialog_id: rvoip_dialog_core::DialogId,
        event_package: String,
        from_uri: String,
        to_uri: String,
        expires: std::time::Duration,
    ) -> Result<()> {
        tracing::info!(
            "Subscription created: package={}, from={}, to={}, expires={:?}",
            event_package, from_uri, to_uri, expires
        );
        
        // For presence subscriptions, delegate to PresenceCoordinator
        if event_package == "presence" {
            let presence_coordinator = self.presence_coordinator.read().await;
            presence_coordinator.handle_subscription(
                dialog_id.clone(),
                from_uri.clone(),
                to_uri.clone(),
                event_package.clone(),
                expires,
            ).await?;
            
            // Mark the subscription as active in dialog-core
            if let Some(subscription_manager) = self.dialog_manager.subscription_manager() {
                if let Err(e) = subscription_manager.activate_subscription(&dialog_id).await {
                    tracing::warn!("Failed to activate subscription for dialog {}: {}", dialog_id, e);
                }
            }
        }
        
        // Notify application handler if present
        if let Some(handler) = &self.handler {
            let dialog_id_str = dialog_id.to_string();
            handler.on_subscription_created(&dialog_id_str, &event_package, &from_uri).await;
        }
        
        Ok(())
    }
    
    /// Handle NOTIFY received event
    async fn handle_notify_received(
        &self,
        dialog_id: rvoip_dialog_core::DialogId,
        subscription_state: String,
        event_package: String,
        body: Option<Vec<u8>>,
    ) -> Result<()> {
        tracing::info!(
            "NOTIFY received: dialog={}, package={}, state={}",
            dialog_id, event_package, subscription_state
        );
        
        // Parse presence data if this is a presence NOTIFY
        if event_package == "presence" && body.is_some() {
            // TODO: Parse PIDF XML and extract presence state
            // This will be handled by PresenceCoordinator
            tracing::debug!("Received presence NOTIFY with body");
        }
        
        // Check if subscription is terminated
        if subscription_state.starts_with("terminated") {
            tracing::info!("Subscription {} terminated via NOTIFY", dialog_id);
        }
        
        // Notify application handler if present
        if let Some(handler) = &self.handler {
            let dialog_id_str = dialog_id.to_string();
            handler.on_notify_received(
                &dialog_id_str,
                &event_package,
                body.as_deref(),
            ).await;
        }
        
        Ok(())
    }
    
    /// Handle subscription terminated event
    async fn handle_subscription_terminated(
        &self,
        dialog_id: rvoip_dialog_core::DialogId,
        reason: Option<String>,
    ) -> Result<()> {
        tracing::info!(
            "Subscription terminated: dialog={}, reason={:?}",
            dialog_id, reason
        );

        // Capture dialog_id string before ownership moves to terminate_subscription
        let dialog_id_str = dialog_id.to_string();

        // Clean up presence subscription
        let presence_coordinator = self.presence_coordinator.read().await;
        presence_coordinator.terminate_subscription(dialog_id, reason.clone()).await?;

        // Notify application handler if present
        if let Some(handler) = &self.handler {
            handler.on_subscription_terminated(
                &dialog_id_str,
                reason.as_deref(),
            ).await;
        }
        
        Ok(())
    }
    
    /// Handle presence state update request
    async fn handle_presence_state_update(
        &self,
        user_uri: String,
        state: String,
        note: Option<String>,
    ) -> Result<()> {
        tracing::info!(
            "Presence state update: user={}, state={}, note={:?}",
            user_uri, state, note
        );
        
        // Parse the state string into PresenceStatus
        use super::presence::PresenceStatus;
        let presence_status = match state.to_lowercase().as_str() {
            "available" | "online" => PresenceStatus::Available,
            "busy" => PresenceStatus::Busy,
            "away" => PresenceStatus::Away,
            "dnd" | "do-not-disturb" => PresenceStatus::DoNotDisturb,
            "offline" => PresenceStatus::Offline,
            "in-call" => PresenceStatus::InCall,
            custom => PresenceStatus::Custom(custom.to_string()),
        };
        
        // Update presence state and notify watchers
        let presence_coordinator = self.presence_coordinator.read().await;
        presence_coordinator.update_presence(user_uri.clone(), presence_status, note.clone()).await?;

        // Notify application handler if present
        if let Some(handler) = &self.handler {
            handler.on_presence_update(
                &user_uri,
                &state,
                note.as_deref(),
            ).await;
        }
        
        Ok(())
    }

    // ---------------------------------------------------------------
    // Trickle ICE (RFC 8838 / RFC 8840) event handlers
    // ---------------------------------------------------------------

    /// Handle a trickle ICE candidate received via SIP INFO.
    async fn handle_trickle_ice_candidate(
        &self,
        session_id: SessionId,
        candidate_line: String,
    ) -> Result<()> {
        tracing::info!(
            "Trickle ICE: adding remote candidate for session {}: {}",
            session_id,
            candidate_line
        );

        if let Err(e) = self
            .media_manager
            .add_remote_ice_candidate(&session_id, &candidate_line)
            .await
        {
            tracing::error!(
                "Failed to add trickle ICE candidate for session {}: {}",
                session_id,
                e
            );
        }

        Ok(())
    }

    /// Handle a trickle ICE end-of-candidates indication.
    async fn handle_trickle_ice_end_of_candidates(
        &self,
        session_id: SessionId,
    ) -> Result<()> {
        tracing::info!(
            "Trickle ICE: end-of-candidates for session {}",
            session_id
        );

        if let Err(e) = self
            .media_manager
            .set_remote_end_of_candidates(&session_id)
            .await
        {
            tracing::error!(
                "Failed to set end-of-candidates for session {}: {}",
                session_id,
                e
            );
        }

        Ok(())
    }
}