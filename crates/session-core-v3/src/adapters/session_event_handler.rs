//! Session Event Handler - Central hub for ALL cross-crate event handling
//!
//! This is the ONLY place where cross-crate events are handled.
//! - Receives events from dialog-core and media-core
//! - Routes them to the state machine
//! - Publishes events to dialog-core and media-core
//!
//! NO OTHER MODULE should interact with the GlobalEventCoordinator directly.

use std::sync::Arc;
use anyhow::Result;
use tokio::sync::mpsc;
use rvoip_infra_common::events::coordinator::{CrossCrateEventHandler, GlobalEventCoordinator};
use rvoip_infra_common::events::cross_crate::CrossCrateEvent;
use crate::state_table::types::{SessionId, EventType, Role};
use crate::state_machine::StateMachine as StateMachineExecutor;
use crate::errors::{SessionError, Result as SessionResult};
use crate::adapters::{DialogAdapter, MediaAdapter};
use crate::session_registry::SessionRegistry;
use crate::types::DialogId;
use tracing::{debug, info, error, warn};

/// Handler for processing cross-crate events in session-core-v3
#[derive(Clone)]
#[allow(dead_code)]
pub struct SessionCrossCrateEventHandler {
    /// State machine executor
    state_machine: Arc<StateMachineExecutor>,

    /// Global event coordinator
    global_coordinator: Arc<GlobalEventCoordinator>,

    /// Dialog adapter for setting up backward compatibility channels
    dialog_adapter: Arc<DialogAdapter>,

    /// Media adapter for setting up backward compatibility channels
    media_adapter: Arc<MediaAdapter>,

    /// Session registry for mappings
    registry: Arc<SessionRegistry>,

    /// Channel to send incoming call notifications
    incoming_call_tx: Option<mpsc::Sender<crate::types::IncomingCallInfo>>,

}

impl SessionCrossCrateEventHandler {
    pub fn new(
        state_machine: Arc<StateMachineExecutor>,
        global_coordinator: Arc<GlobalEventCoordinator>,
        dialog_adapter: Arc<DialogAdapter>,
        media_adapter: Arc<MediaAdapter>,
        registry: Arc<SessionRegistry>,
    ) -> Self {
        Self {
            state_machine,
            global_coordinator,
            dialog_adapter,
            media_adapter,
            registry,
            incoming_call_tx: None,
        }
    }

    pub fn with_incoming_call_channel(
        state_machine: Arc<StateMachineExecutor>,
        global_coordinator: Arc<GlobalEventCoordinator>,
        dialog_adapter: Arc<DialogAdapter>,
        media_adapter: Arc<MediaAdapter>,
        registry: Arc<SessionRegistry>,
        incoming_call_tx: mpsc::Sender<crate::types::IncomingCallInfo>,
    ) -> Self {
        Self {
            state_machine,
            global_coordinator,
            dialog_adapter,
            media_adapter,
            registry,
            incoming_call_tx: Some(incoming_call_tx),
        }
    }

    /// Preferred constructor — events are published to the global coordinator's
    /// "session_to_app" channel automatically; no separate broadcast sender needed.
    pub fn with_event_broadcast(
        state_machine: Arc<StateMachineExecutor>,
        global_coordinator: Arc<GlobalEventCoordinator>,
        dialog_adapter: Arc<DialogAdapter>,
        media_adapter: Arc<MediaAdapter>,
        registry: Arc<SessionRegistry>,
        incoming_call_tx: mpsc::Sender<crate::types::IncomingCallInfo>,
    ) -> Self {
        Self::with_incoming_call_channel(
            state_machine,
            global_coordinator,
            dialog_adapter,
            media_adapter,
            registry,
            incoming_call_tx,
        )
    }

    /// Deprecated: use `with_event_broadcast` instead.
    #[deprecated(note = "Use with_event_broadcast")]
    pub fn with_simple_peer_events(
        state_machine: Arc<StateMachineExecutor>,
        global_coordinator: Arc<GlobalEventCoordinator>,
        dialog_adapter: Arc<DialogAdapter>,
        media_adapter: Arc<MediaAdapter>,
        registry: Arc<SessionRegistry>,
        incoming_call_tx: mpsc::Sender<crate::types::IncomingCallInfo>,
        _simple_peer_event_tx: tokio::sync::mpsc::Sender<crate::api::events::Event>,
    ) -> Self {
        Self::with_incoming_call_channel(
            state_machine,
            global_coordinator,
            dialog_adapter,
            media_adapter,
            registry,
            incoming_call_tx,
        )
    }
    
    /// Publish a terminal app-level event, then release the session from the
    /// store + registry.
    ///
    /// Terminal events are `CallEnded`, `CallFailed`, `CallCancelled`. Publish
    /// runs first so any subscriber that queries session state in response to
    /// the event still sees a populated entry; the release then happens in the
    /// same spawned task after publish returns. Without this, long-running
    /// peers (and especially b2bua, which multiplies sessions) would leak
    /// `SessionStore` entries indefinitely.
    async fn publish_and_release_session(
        &self,
        api_event: crate::api::events::Event,
        session_id: SessionId,
    ) {
        let wrapped = crate::adapters::SessionApiCrossCrateEvent::new(api_event);
        let coordinator = self.global_coordinator.clone();
        let store = self.state_machine.store.clone();
        let registry = self.registry.clone();
        tokio::spawn(async move {
            if let Err(e) = coordinator.publish(wrapped).await {
                tracing::warn!("Failed to publish terminal event to global coordinator: {}", e);
            }
            if let Err(e) = store.remove_session(&session_id).await {
                // Not-found is expected if another terminal path got there
                // first — log at debug only.
                tracing::debug!("remove_session({}) during terminal cleanup: {}", session_id, e);
            }
            registry.remove_session(&session_id).await;
        });
    }

    /// Start event processing loops.
    ///
    /// Background tasks will stop when `shutdown_rx` receives `true`.
    pub async fn start(&self, shutdown_rx: tokio::sync::watch::Receiver<bool>) -> SessionResult<()> {
        self.start_global_event_subscriptions(shutdown_rx).await?;
        Ok(())
    }

    /// Start subscriptions to global cross-crate events
    async fn start_global_event_subscriptions(
        &self,
        shutdown_rx: tokio::sync::watch::Receiver<bool>,
    ) -> SessionResult<()> {
        // Subscribe to dialog-to-session events
        let mut dialog_sub = self.global_coordinator
            .subscribe("dialog_to_session")
            .await
            .map_err(|e| SessionError::InternalError(format!("Failed to subscribe to dialog events: {}", e)))?;

        let handler = self.clone();
        let mut shutdown = shutdown_rx.clone();
        tokio::spawn(async move {
            info!("🔔 [session_event_handler] Started dialog-to-session event loop");
            loop {
                tokio::select! {
                    _ = shutdown.changed() => {
                        if *shutdown.borrow() {
                            info!("🔔 [session_event_handler] Dialog event loop shutting down");
                            break;
                        }
                    }
                    event = dialog_sub.recv() => {
                        let Some(event) = event else { break };
                        info!("🔔 [session_event_handler] Received event from channel: {:?}", event);
                        if let Err(e) = handler.handle(event).await {
                            error!("Error handling dialog-to-session event: {}", e);
                        }
                    }
                }
            }
            info!("🔔 [session_event_handler] Dialog-to-session event loop ended");
        });

        // Subscribe to media-to-session events
        let mut media_sub = self.global_coordinator
            .subscribe("media_to_session")
            .await
            .map_err(|e| SessionError::InternalError(format!("Failed to subscribe to media events: {}", e)))?;

        let handler = self.clone();
        let mut shutdown = shutdown_rx.clone();
        tokio::spawn(async move {
            loop {
                tokio::select! {
                    _ = shutdown.changed() => {
                        if *shutdown.borrow() { break; }
                    }
                    event = media_sub.recv() => {
                        let Some(event) = event else { break };
                        if let Err(e) = handler.handle(event).await {
                            error!("Error handling media-to-session event: {}", e);
                        }
                    }
                }
            }
        });

        Ok(())
    }
}

#[async_trait::async_trait]
impl CrossCrateEventHandler for SessionCrossCrateEventHandler {
    async fn handle(&self, event: Arc<dyn CrossCrateEvent>) -> Result<()> {
        debug!("Handling cross-crate event: {}", event.event_type());
        
        // Note: Downcasting Arc<dyn CrossCrateEvent> to concrete types would require
        // additional trait bounds (like Any) and type registration. For now, we use
        // string parsing of the debug representation as a pragmatic workaround.
        // This is acceptable because:
        // 1. Events are internal to the system (not user-facing)
        // 2. Debug representations are stable within our codebase
        // 3. Performance impact is minimal (events are not high-frequency)
        let event_str = format!("{:?}", event);
        
        match event.event_type() {
            "dialog_to_session" => {
                info!("Processing dialog-to-session event");
                
                // Parse the debug output to determine the specific event variant
                if event_str.contains("DialogCreated") {
                    self.handle_dialog_created(&event_str).await?;
                } else if event_str.contains("IncomingCall") {
                    self.handle_incoming_call(&event_str).await?;
                } else if event_str.contains("CallEstablished") {
                    self.handle_call_established(&event_str).await?;
                } else if event_str.contains("CallCancelled") {
                    self.handle_call_cancelled(&event_str).await?;
                } else if event_str.contains("CallRedirected") {
                    self.handle_call_redirected(&event_str).await?;
                } else if event_str.contains("ReinviteGlare") {
                    self.handle_reinvite_glare(&event_str).await?;
                } else if event_str.contains("SessionRefreshFailed") {
                    // Check this BEFORE SessionRefreshed — the latter is a
                    // substring of the former and `contains()` would match
                    // either.
                    self.handle_session_refresh_failed(&event_str).await?;
                } else if event_str.contains("SessionRefreshed") {
                    self.handle_session_refreshed(&event_str).await?;
                } else if event_str.contains("AuthRequired") {
                    // Check BEFORE CallFailed — both could otherwise match via
                    // substring on future enrichment.
                    self.handle_auth_required(&event_str).await?;
                } else if event_str.contains("SessionIntervalTooSmall") {
                    // RFC 4028 §6 — 422 retry with bumped Session-Expires.
                    // Check BEFORE CallFailed so a substring match on
                    // "Failed" (future enrichment) won't swallow it.
                    self.handle_session_interval_too_small(&event_str).await?;
                } else if event_str.contains("CallFailed") {
                    self.handle_call_failed(&event_str).await?;
                } else if event_str.contains("CallStateChanged") {
                    self.handle_call_state_changed(&event_str).await?;
                } else if event_str.contains("CallTerminated") {
                    self.handle_call_terminated(&event_str).await?;
                } else if event_str.contains("DialogError") {
                    self.handle_dialog_error(&event_str).await?;
                } else if event_str.contains("DialogStateChanged") {
                    self.handle_dialog_state_changed(&event_str).await?;
                } else if event_str.contains("ReinviteReceived") {
                    self.handle_reinvite_received(&event_str).await?;
                } else if event_str.contains("TransferRequested") {
                    self.handle_transfer_requested(&event_str).await?;
                } else if event_str.contains("AckSent") {
                    self.handle_ack_sent(&event_str).await?;
                } else if event_str.contains("AckReceived") {
                    self.handle_ack_received(&event_str).await?;
                } else {
                    debug!("Unhandled dialog-to-session event: {}", event_str);
                }
            }
            "media_to_session" => {
                info!("Processing media-to-session event");
                
                // Parse the debug output to determine the specific event variant
                if event_str.contains("MediaStreamStarted") {
                    self.handle_media_stream_started(&event_str).await?;
                } else if event_str.contains("MediaStreamStopped") {
                    self.handle_media_stream_stopped(&event_str).await?;
                } else if event_str.contains("MediaFlowEstablished") {
                    self.handle_media_flow_established(&event_str).await?;
                } else if event_str.contains("MediaError") {
                    self.handle_media_error(&event_str).await?;
                } else if event_str.contains("MediaQualityDegraded") {
                    self.handle_media_quality_degraded(&event_str).await?;
                } else if event_str.contains("DtmfDetected") {
                    self.handle_dtmf_detected(&event_str).await?;
                } else if event_str.contains("RtpTimeout") {
                    self.handle_rtp_timeout(&event_str).await?;
                } else if event_str.contains("PacketLossThresholdExceeded") {
                    self.handle_packet_loss_threshold_exceeded(&event_str).await?;
                }
            }
            _ => {
                debug!("Unhandled event type: {}", event.event_type());
            }
        }
        
        Ok(())
    }
}

impl SessionCrossCrateEventHandler {
    
    /// Check if a session belongs to this handler's store.
    /// Returns false (and logs at debug) if the session was created by a different peer.
    async fn is_our_session(&self, session_id: &SessionId) -> bool {
        self.state_machine.store.get_session(session_id).await.is_ok()
    }

    /// Extract session ID from event debug string (temporary workaround)
    fn extract_session_id(&self, event_str: &str) -> Option<String> {
        // Look for session_id in the debug output
        if let Some(start) = event_str.find("session_id: \"") {
            let start = start + 13;
            if let Some(end) = event_str[start..].find('"') {
                let session_id = event_str[start..start+end].to_string();
                info!("✅ [extract_session_id] Successfully extracted: {}", session_id);
                return Some(session_id);
            }
        }
        warn!("⚠️ [extract_session_id] Failed to extract session_id from event: {}", 
              if event_str.len() > 200 { &event_str[..200] } else { event_str });
        None
    }
    
    /// Extract a field value from event debug string (temporary workaround)
    fn extract_field(&self, event_str: &str, field_prefix: &str) -> Option<String> {
        if let Some(start) = event_str.find(field_prefix) {
            let start = start + field_prefix.len();
            if let Some(end) = event_str[start..].find('"') {
                return Some(event_str[start..start+end].to_string());
            }
        }
        None
    }
    
    
    // Dialog event handlers
    async fn handle_dialog_created(&self, event_str: &str) -> Result<()> {
        // Extract dialog_id and call_id
        let dialog_id = self.extract_field(event_str, "dialog_id: \"").unwrap_or_else(|| "unknown".to_string());
        let call_id = self.extract_field(event_str, "call_id: \"").unwrap_or_else(|| "unknown".to_string());

        // Check if this is our call (session-core generated Call-ID)
        if call_id.contains("@session-core") {
            if let Some(session_id_str) = call_id.split('@').next() {
                let session_id = SessionId(session_id_str.to_string());

                // Check if session exists before processing event
                // DialogCreated may arrive before the MakeCall transition completes
                if self.state_machine.store.get_session(&session_id).await.is_err() {
                    debug!("DialogCreated event arrived before session {} was fully created, will be handled by state machine later", session_id);
                    return Ok(());
                }

                // Only trigger state transition - all logic should be in the state machine
                if let Err(e) = self.state_machine.process_event(
                    &session_id,
                    EventType::DialogCreated { dialog_id, call_id }
                ).await {
                    error!("Failed to process DialogCreated event: {}", e);
                }
            }
        }

        Ok(())
    }
    
    async fn handle_incoming_call(&self, event_str: &str) -> Result<()> {
        // Extract fields from the event
        // Extract session_id from the event (dialog-core provides it)
        let session_id_str = self.extract_field(event_str, "session_id: \"").unwrap_or_else(|| format!("session-{}", uuid::Uuid::new_v4()));

        // Extract dialog_id from headers since IncomingCall doesn't have a dialog_id field directly
        let dialog_id_str = if let Some(headers_start) = event_str.find("headers: {") {
            // Look for X-Dialog-Id in headers
            let headers_section = &event_str[headers_start..];
            if let Some(dialog_id_start) = headers_section.find("\"X-Dialog-Id\": \"") {
                let start = dialog_id_start + "\"X-Dialog-Id\": \"".len();
                if let Some(end) = headers_section[start..].find('"') {
                    headers_section[start..start+end].to_string()
                } else {
                    "unknown".to_string()
                }
            } else {
                "unknown".to_string()
            }
        } else {
            "unknown".to_string()
        };

        // IMPORTANT: Check if this event is for OUR dialog instance.
        // Multiple peers in the same process share a GlobalEventCoordinator,
        // so every handler receives every IncomingCall event. We must only
        // process the event if the dialog was created by OUR dialog-core.
        if let Ok(dialog_uuid) = uuid::Uuid::parse_str(&dialog_id_str) {
            let rvoip_dialog_id = rvoip_dialog_core::DialogId(dialog_uuid);

            // Check if this dialog exists in our dialog adapter's session_to_dialog map
            // If the dialog is already mapped, it means another peer is handling it
            if self.dialog_adapter.dialog_to_session.contains_key(&rvoip_dialog_id) {
                debug!("Ignoring IncomingCall for dialog {} - already handled by another peer", dialog_id_str);
                return Ok(());
            }

            // Check if this dialog exists in our own dialog-core instance.
            // If it doesn't, the INVITE was received by a different peer's
            // dialog-core and we must not try to process it.
            if !self.dialog_adapter.dialog_api.dialog_manager().core().has_dialog(&rvoip_dialog_id) {
                debug!("Ignoring IncomingCall for dialog {} - not in our dialog-core", dialog_id_str);
                return Ok(());
            }
        }

        let call_id = self.extract_field(event_str, "call_id: \"").unwrap_or_else(|| "unknown".to_string());
        let from = self.extract_field(event_str, "from: \"").unwrap_or_else(|| "unknown".to_string());
        let to = self.extract_field(event_str, "to: \"").unwrap_or_else(|| "unknown".to_string());
        let sdp = self.extract_field(event_str, "sdp_offer: Some(\"")
            .map(|s| s.replace("\\r\\n", "\r\n").replace("\\n", "\n").replace("\\\"", "\""));
        let _transaction_id = self.extract_field(event_str, "transaction_id: \"").unwrap_or_else(|| "unknown".to_string());
        let _source_addr = self.extract_field(event_str, "source_addr: \"").unwrap_or_else(|| "127.0.0.1:5060".to_string());
        
        // Use the session ID provided by dialog-core
        let session_id = SessionId(session_id_str);

        // Create session in store - this is the ONLY place we create sessions outside state machine
        self.state_machine.store.create_session(
            session_id.clone(),
            Role::UAS,
            true,
        ).await.map_err(|e| SessionError::InternalError(format!("Failed to create session: {}", e)))?;

        // IMPORTANT: Populate the session with URIs before processing events
        // The state machine's CreateDialog action requires these fields
        let mut session = self.state_machine.store.get_session(&session_id).await
            .map_err(|e| SessionError::InternalError(format!("Failed to get newly created session: {}", e)))?;
        session.local_uri = Some(to.clone());    // The "To" header is us (answerer)
        session.remote_uri = Some(from.clone()); // The "From" header is the caller
        
        // Store session data for SimplePeer event
        let session_remote_sdp = session.remote_sdp.clone();
        
        self.state_machine.store.update_session(session).await
            .map_err(|e| SessionError::InternalError(format!("Failed to update session URIs: {}", e)))?;

        // Parse dialog UUID for registry mapping
        let dialog_uuid = uuid::Uuid::parse_str(&dialog_id_str).unwrap_or_else(|_| uuid::Uuid::new_v4());
        
        // Store mapping info for state machine to use
        self.registry.map_dialog(session_id.clone(), DialogId(dialog_uuid)).await;
        self.registry.store_pending_incoming_call(
            session_id.clone(),
            crate::types::IncomingCallInfo {
                session_id: session_id.clone(),
                from: from.clone(),
                to: to.clone(),
                        call_id: call_id.clone(),
                dialog_id: DialogId(dialog_uuid),
            }
        ).await;
        
        // Store the mapping in dialog adapter for local reference
        // Convert our DialogId to rvoip DialogId
        let our_dialog_id = DialogId(dialog_uuid);
        let rvoip_dialog_id = rvoip_dialog_core::DialogId::from(our_dialog_id.clone());
        self.dialog_adapter.session_to_dialog.insert(session_id.clone(), rvoip_dialog_id.clone());
        self.dialog_adapter.dialog_to_session.insert(rvoip_dialog_id.clone(), session_id.clone());

        // IMPORTANT: Publish StoreDialogMapping so dialog-core can route session-based operations
        // Dialog-core needs this for send_response_for_session() to work
        let event = rvoip_infra_common::events::cross_crate::SessionToDialogEvent::StoreDialogMapping {
            session_id: session_id.0.clone(),
            dialog_id: dialog_uuid.to_string(),
        };
        if let Err(e) = self.dialog_adapter.global_coordinator.publish(Arc::new(
            rvoip_infra_common::events::cross_crate::RvoipCrossCrateEvent::SessionToDialog(event)
        )).await {
            error!("Failed to publish StoreDialogMapping for UAS: {}", e);
        }

        // Process the event - state machine will handle the rest
        let event_type = EventType::IncomingCall { from: from.clone(), sdp };
        
        if let Err(e) = self.state_machine.process_event(
            &session_id,
            event_type
        ).await {
            error!("Failed to process incoming call event: {}", e);
            // Clean up on failure
            let _ = self.state_machine.store.remove_session(&session_id).await;
            self.registry.remove_session(&session_id).await;
        } else {
            // Publish IncomingCall event to the global coordinator's "session_to_app" channel.
            // All active subscribers (StreamPeer, CallbackPeer, etc.) will receive it.
            {
                debug!("🔍 [DEBUG] Publishing IncomingCall event to global coordinator");
                let api_event = crate::api::events::Event::IncomingCall {
                    call_id: session_id.clone(),
                    from: from.clone(),
                    to: to.clone(),
                    sdp: session_remote_sdp,
                };
                let wrapped = crate::adapters::SessionApiCrossCrateEvent::new(api_event);
                let coordinator = self.global_coordinator.clone();
                tokio::spawn(async move {
                    if let Err(e) = coordinator.publish(wrapped).await {
                        tracing::warn!("Failed to publish IncomingCall to global coordinator: {}", e);
                    }
                });
            }
            
            // Legacy incoming call notification (keep for compatibility)
            if let Some(ref tx) = self.incoming_call_tx {
                info!("Sending incoming call notification for session {}", session_id);
                let call_info = crate::types::IncomingCallInfo {
                    session_id: session_id.clone(),
                    from,
                    to,
                    call_id,
                    dialog_id: DialogId(dialog_uuid),
                };
                if let Err(e) = tx.send(call_info).await {
                    error!("Failed to send incoming call notification: {}", e);
                } else {
                    info!("Successfully sent incoming call notification");
                }
            } else {
                warn!("No incoming_call_tx channel available to send notification");
            }
        }
        
        Ok(())
    }
    
    async fn handle_call_established(&self, event_str: &str) -> Result<()> {
        info!("🎯 [handle_call_established] Called with event: {}", event_str);

        // Extract session_id field from event
        // Dialog-core's event_hub retrieves the actual session_id via dialog_manager.get_session_id()
        // This is the real session ID in "session-XXX" format, not a dialog_id!
        let session_id_str = self.extract_session_id(event_str).unwrap_or_else(|| "unknown".to_string());

        info!("🎯 [handle_call_established] Extracted session_id: {}", session_id_str);

        if session_id_str == "unknown" {
            error!("Cannot extract session_id from CallEstablished event");
            return Ok(());
        }

        let session_id = SessionId(session_id_str);

        // Skip if this session isn't ours — multiple peers share the global event bus
        if self.state_machine.store.get_session(&session_id).await.is_err() {
            debug!("Ignoring CallEstablished for session {} - not in our store", session_id);
            return Ok(());
        }

        info!("🎯 [handle_call_established] Processing CallEstablished for session {}", session_id);

        let sdp_answer = self.extract_field(event_str, "sdp_answer: Some(\"")
            .map(|s| s.replace("\\r\\n", "\r\n").replace("\\n", "\n").replace("\\\"", "\""));

        // Store remote SDP if present
        if let Some(sdp) = &sdp_answer {
            info!("Stored remote SDP from CallEstablished for session {}", session_id);
            // Update the session with remote SDP
            if let Ok(mut session) = self.state_machine.store.get_session(&session_id).await {
                session.remote_sdp = Some(sdp.clone());
                let _ = self.state_machine.store.update_session(session).await;
            }
        }

        // CallEstablished maps to Dialog200OK for state machine processing
        if let Err(e) = self.state_machine.process_event(
            &session_id,
            EventType::Dialog200OK
        ).await {
            error!("Failed to process CallEstablished as Dialog200OK: {}", e);
        }

        // Publish CallAnswered event to the global coordinator's "session_to_app" channel.
        {
            debug!("🔍 [DEBUG] Publishing CallAnswered event to global coordinator");
            let api_event = crate::api::events::Event::CallAnswered {
                call_id: session_id.clone(),
                sdp: sdp_answer,
            };
            let wrapped = crate::adapters::SessionApiCrossCrateEvent::new(api_event);
            let coordinator = self.global_coordinator.clone();
            tokio::spawn(async move {
                if let Err(e) = coordinator.publish(wrapped).await {
                    tracing::warn!("Failed to publish CallAnswered to global coordinator: {}", e);
                }
            });
        }

        Ok(())
    }

    /// Handle a 401/407 digest auth challenge (RFC 3261 §22.2) surfaced by
    /// dialog-core as `DialogToSessionEvent::AuthRequired`. Parses the raw
    /// challenge + status from the debug-formatted event string and drives
    /// the state machine through the shared `AuthRequired` transition. The
    /// action layer (`StoreAuthChallenge` + `SendINVITEWithAuth` /
    /// `SendREGISTERWithAuth`) takes it from there.
    ///
    /// Method-agnostic: session state (`Initiating` / `Registering`)
    /// disambiguates whether this retries INVITE or REGISTER.
    async fn handle_auth_required(&self, event_str: &str) -> Result<()> {
        let Some(session_id_str) = self.extract_session_id(event_str) else {
            warn!("Could not extract session_id from AuthRequired event");
            return Ok(());
        };
        let session_id = SessionId(session_id_str);

        if !self.is_our_session(&session_id).await {
            debug!("Ignoring AuthRequired for session {} - not in our store", session_id);
            return Ok(());
        }

        let status = self
            .extract_field(event_str, "status_code: ")
            .and_then(|s| s.split(|c: char| !c.is_ascii_digit()).next().and_then(|n| n.parse::<u16>().ok()))
            .unwrap_or(401);
        let challenge = self
            .extract_field(event_str, "challenge: \"")
            .unwrap_or_default();

        info!(
            "🎯 [handle_auth_required] session={} status={} challenge.len={}",
            session_id,
            status,
            challenge.len()
        );

        if let Err(e) = self
            .state_machine
            .process_event(&session_id, EventType::AuthRequired { status_code: status, challenge })
            .await
        {
            error!("Failed to process AuthRequired({}) for session {}: {}", status, session_id, e);
        }
        Ok(())
    }

    /// Handle a 3xx/4xx/5xx/6xx final failure response for an outgoing request.
    /// Drives the state machine through the appropriate `Dialog{4,5,6}xxFailure`
    /// transition and publishes an app-level `CallFailed` event so peer
    /// subscribers (StreamPeer, CallbackPeer) learn the call was rejected.
    async fn handle_call_failed(&self, event_str: &str) -> Result<()> {
        let Some(session_id_str) = self.extract_session_id(event_str) else {
            warn!("Could not extract session_id from CallFailed event");
            return Ok(());
        };
        let session_id = SessionId(session_id_str);

        if !self.is_our_session(&session_id).await {
            debug!("Ignoring CallFailed for session {} - not in our store", session_id);
            return Ok(());
        }

        let status = self
            .extract_field(event_str, "status_code: ")
            .and_then(|s| s.split(|c: char| !c.is_ascii_digit()).next().and_then(|n| n.parse::<u16>().ok()))
            .unwrap_or(500);
        let reason = self
            .extract_field(event_str, "reason_phrase: \"")
            .unwrap_or_else(|| "Failure".to_string());

        info!("🎯 [handle_call_failed] session={} status={} reason={}", session_id, status, reason);

        // Drive the existing Dialog{4,5,6}xxFailure state transitions. 3xx
        // currently maps onto the 4xx path because the default state table
        // has no dedicated redirect transition; proper 3xx/redirect handling
        // is a separate feature.
        let event_type = match status {
            300..=499 => EventType::Dialog4xxFailure(status),
            500..=599 => EventType::Dialog5xxFailure(status),
            600..=699 => EventType::Dialog6xxFailure(status),
            _ => EventType::DialogError(format!("unexpected CallFailed status {}", status)),
        };

        if let Err(e) = self.state_machine.process_event(&session_id, event_type).await {
            error!("Failed to process CallFailed({}) for session {}: {}", status, session_id, e);
        }

        // Publish app-level CallFailed for any StreamPeer/CallbackPeer subscribers,
        // then release the session from the store + registry. Publish runs first
        // so subscribers receive the terminal event before the session vanishes.
        let api_event = crate::api::events::Event::CallFailed {
            call_id: session_id.clone(),
            status_code: status,
            reason: reason.clone(),
        };
        self.publish_and_release_session(api_event, session_id.clone()).await;

        Ok(())
    }

    /// Handle a 3xx redirect response (RFC 3261 §8.1.3.4). Parses the
    /// `targets: [...]` list from the debug-formatted event and passes it
    /// to the state machine's `Dialog3xxRedirect` transition, which runs
    /// `RetryWithContact` to re-send INVITE to the first URI.
    async fn handle_call_redirected(&self, event_str: &str) -> Result<()> {
        let Some(session_id_str) = self.extract_session_id(event_str) else {
            warn!("Could not extract session_id from CallRedirected event");
            return Ok(());
        };
        let session_id = SessionId(session_id_str);

        if !self.is_our_session(&session_id).await {
            debug!("Ignoring CallRedirected for session {} - not in our store", session_id);
            return Ok(());
        }

        let status = self
            .extract_field(event_str, "status_code: ")
            .and_then(|s| s.split(|c: char| !c.is_ascii_digit()).next().and_then(|n| n.parse::<u16>().ok()))
            .unwrap_or(302);

        // targets comes across as `targets: ["sip:a@...", "sip:b@..."]`. Pull
        // the first balanced `[...]` out, then extract the quoted URIs.
        let targets: Vec<String> = if let Some(start) = event_str.find("targets: [") {
            let after = &event_str[start + "targets: [".len()..];
            if let Some(end) = after.find(']') {
                let inner = &after[..end];
                inner
                    .split(',')
                    .filter_map(|frag| {
                        let f = frag.trim();
                        f.strip_prefix('"')
                            .and_then(|s| s.strip_suffix('"'))
                            .map(|s| s.to_string())
                    })
                    .collect()
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };

        info!(
            "🔀 [handle_call_redirected] session={} status={} targets={:?}",
            session_id, status, targets
        );

        if targets.is_empty() {
            // No usable contacts — treat as a 4xx failure.
            warn!("3xx response with no Contact URIs — treating as failure");
            let _ = self
                .state_machine
                .process_event(&session_id, EventType::Dialog4xxFailure(status))
                .await;
            return Ok(());
        }

        if let Err(e) = self
            .state_machine
            .process_event(
                &session_id,
                EventType::Dialog3xxRedirect { status, targets },
            )
            .await
        {
            error!("Failed to process CallRedirected for session {}: {}", session_id, e);
        }

        Ok(())
    }

    /// Handle RFC 4028 §6 — 422 Session Interval Too Small. The UAS requires
    /// a session interval larger than we offered; its `Min-SE:` header
    /// (surfaced as `min_se_secs`) carries the floor.
    ///
    /// RFC 4028 §6 — UAS replied 422 Session Interval Too Small. Two paths:
    ///
    /// 1. **Auto-retry** (usual path): if the response carries a parseable
    ///    `Min-SE` and the session's retry counter is below the cap, dispatch
    ///    `SessionIntervalTooSmall { min_se_secs }` to the state machine.
    ///    `SendINVITEWithBumpedSessionExpires` re-issues the INVITE with the
    ///    peer's floor and the 2-retry cap lives in that action.
    ///
    /// 2. **Terminal fallback**: when `Min-SE` is missing/zero or the retry
    ///    cap has already been hit, route through the generic
    ///    `Dialog4xxFailure(422)` path and publish a terminal `CallFailed`
    ///    so the app can observe the 422 status. Mirrors how dialog-core's
    ///    `event_hub.rs` already degrades gracefully on malformed 422s.
    async fn handle_session_interval_too_small(&self, event_str: &str) -> Result<()> {
        let Some(session_id_str) = self.extract_session_id(event_str) else {
            warn!("Could not extract session_id from SessionIntervalTooSmall event");
            return Ok(());
        };
        let session_id = SessionId(session_id_str);

        if !self.is_our_session(&session_id).await {
            debug!(
                "Ignoring SessionIntervalTooSmall for session {} - not in our store",
                session_id
            );
            return Ok(());
        }

        // Numeric fields in the Debug output aren't quoted, so extract_field
        // (which expects `"…"`-wrapped string values) returns None. Pull the
        // digits off manually — find "min_se_secs: ", then take the leading
        // run of ASCII digits that follows.
        let min_se_secs = event_str
            .find("min_se_secs: ")
            .and_then(|idx| {
                let start = idx + "min_se_secs: ".len();
                let digits: String = event_str[start..]
                    .chars()
                    .take_while(|c| c.is_ascii_digit())
                    .collect();
                digits.parse::<u32>().ok()
            })
            .unwrap_or(0);

        // Read the retry counter before the state machine runs so we can
        // decide between auto-retry and terminal failure in one place.
        const CAP: u8 = 2;
        let current_retries = self
            .state_machine
            .store
            .get_session(&session_id)
            .await
            .map(|s| s.session_timer_retry_count)
            .unwrap_or(CAP);
        let can_retry = min_se_secs > 0 && current_retries < CAP;

        if can_retry {
            info!(
                "⏱️  [422 Session Interval Too Small] session={} requires Min-SE={}s — retrying (attempt {}/{})",
                session_id, min_se_secs, current_retries + 1, CAP
            );
            if let Err(e) = self
                .state_machine
                .process_event(
                    &session_id,
                    EventType::SessionIntervalTooSmall { min_se_secs },
                )
                .await
            {
                // Retry dispatch failed — surface as terminal 422. No
                // `CallFailed` publish needed; the error path below does it.
                error!(
                    "Failed to dispatch SessionIntervalTooSmall retry for session {}: {}",
                    session_id, e
                );
            } else {
                // Successful retry dispatched — don't publish CallFailed.
                // The retry will either succeed (Dialog200OK) or re-enter
                // this handler on a second 422.
                return Ok(());
            }
        } else {
            warn!(
                "⏱️  [422 Session Interval Too Small] session={} — giving up (min_se={}s, retries={}/{}), surfacing as CallFailed",
                session_id, min_se_secs, current_retries, CAP
            );
        }

        // Terminal path: route through generic 4xx failure + publish
        // CallFailed so the session cleans up and the app observes the 422.
        if let Err(e) = self
            .state_machine
            .process_event(&session_id, EventType::Dialog4xxFailure(422))
            .await
        {
            error!(
                "Failed to process 422 SessionIntervalTooSmall fallback for session {}: {}",
                session_id, e
            );
        }

        let api_event = crate::api::events::Event::CallFailed {
            call_id: session_id.clone(),
            status_code: 422,
            reason: format!("Session Interval Too Small (required Min-SE: {}s)", min_se_secs),
        };
        self.publish_and_release_session(api_event, session_id.clone()).await;

        Ok(())
    }

    /// Handle 491 Request Pending (RFC 3261 §14.1) on a re-INVITE. The
    /// state machine's ReinviteGlare transition runs ScheduleReinviteRetry,
    /// which sleeps a random interval and re-issues the pending re-INVITE.
    async fn handle_reinvite_glare(&self, event_str: &str) -> Result<()> {
        let Some(session_id_str) = self.extract_session_id(event_str) else {
            warn!("Could not extract session_id from ReinviteGlare event");
            return Ok(());
        };
        let session_id = SessionId(session_id_str);

        if !self.is_our_session(&session_id).await {
            debug!("Ignoring ReinviteGlare for session {} - not in our store", session_id);
            return Ok(());
        }

        info!("🔄 [handle_reinvite_glare] session={} — scheduling re-INVITE retry", session_id);

        if let Err(e) = self
            .state_machine
            .process_event(&session_id, EventType::ReinviteGlare)
            .await
        {
            error!("Failed to process ReinviteGlare for session {}: {}", session_id, e);
        }
        Ok(())
    }

    /// Handle 487 Request Terminated — the caller CANCELed before the UAS
    /// answered. Distinct from the generic failure path so we can publish
    /// `Event::CallCancelled` (distinct "missed call" semantic for UIs).
    async fn handle_session_refreshed(&self, event_str: &str) -> Result<()> {
        let Some(session_id_str) = self.extract_session_id(event_str) else {
            warn!("Could not extract session_id from SessionRefreshed event");
            return Ok(());
        };
        let session_id = SessionId(session_id_str);
        if !self.is_our_session(&session_id).await {
            return Ok(());
        }
        // `extract_field` terminates on the next `"`, which works for quoted
        // string fields but not numeric ones — `expires_secs: 10 })` has no
        // trailing quote, so the helper returns None. Parse the digits directly.
        let expires_secs = event_str
            .find("expires_secs: ")
            .map(|idx| &event_str[idx + "expires_secs: ".len()..])
            .and_then(|rest| {
                let digits: String = rest.chars().take_while(|c| c.is_ascii_digit()).collect();
                digits.parse::<u32>().ok()
            })
            .unwrap_or(0);
        info!("🎯 [handle_session_refreshed] session={} expires={}", session_id, expires_secs);

        let api_event = crate::api::events::Event::SessionRefreshed {
            call_id: session_id.clone(),
            expires_secs,
        };
        let wrapped = crate::adapters::SessionApiCrossCrateEvent::new(api_event);
        let coordinator = self.global_coordinator.clone();
        tokio::spawn(async move {
            if let Err(e) = coordinator.publish(wrapped).await {
                tracing::warn!("Failed to publish SessionRefreshed: {}", e);
            }
        });
        Ok(())
    }

    async fn handle_session_refresh_failed(&self, event_str: &str) -> Result<()> {
        let Some(session_id_str) = self.extract_session_id(event_str) else {
            warn!("Could not extract session_id from SessionRefreshFailed event");
            return Ok(());
        };
        let session_id = SessionId(session_id_str);
        if !self.is_our_session(&session_id).await {
            return Ok(());
        }
        let reason = self
            .extract_field(event_str, "reason: \"")
            .unwrap_or_else(|| "Session expired".to_string());
        warn!("🎯 [handle_session_refresh_failed] session={} reason={}", session_id, reason);

        let api_event = crate::api::events::Event::SessionRefreshFailed {
            call_id: session_id.clone(),
            reason,
        };
        let wrapped = crate::adapters::SessionApiCrossCrateEvent::new(api_event);
        let coordinator = self.global_coordinator.clone();
        tokio::spawn(async move {
            if let Err(e) = coordinator.publish(wrapped).await {
                tracing::warn!("Failed to publish SessionRefreshFailed: {}", e);
            }
        });
        Ok(())
    }

    async fn handle_call_cancelled(&self, event_str: &str) -> Result<()> {
        let Some(session_id_str) = self.extract_session_id(event_str) else {
            warn!("Could not extract session_id from CallCancelled event");
            return Ok(());
        };
        let session_id = SessionId(session_id_str);

        if !self.is_our_session(&session_id).await {
            debug!("Ignoring CallCancelled for session {} - not in our store", session_id);
            return Ok(());
        }

        info!("🎯 [handle_call_cancelled] session={}", session_id);

        // Drive the existing Dialog487RequestTerminated state transition.
        if let Err(e) = self
            .state_machine
            .process_event(&session_id, EventType::Dialog487RequestTerminated)
            .await
        {
            error!("Failed to process CallCancelled for session {}: {}", session_id, e);
        }

        // Publish app-level CallCancelled for StreamPeer/CallbackPeer
        // subscribers, then release the session from the store + registry.
        let api_event = crate::api::events::Event::CallCancelled {
            call_id: session_id.clone(),
        };
        self.publish_and_release_session(api_event, session_id.clone()).await;

        Ok(())
    }

    async fn handle_call_state_changed(&self, event_str: &str) -> Result<()> {
        if let Some(session_id) = self.extract_session_id(event_str) {
            let sid = SessionId(session_id);
            if self.state_machine.store.get_session(&sid).await.is_err() {
                debug!("Ignoring CallStateChanged for session {} - not in our store", sid);
                return Ok(());
            }
            if event_str.contains("Ringing") {
                if let Err(e) = self.state_machine.process_event(&sid, EventType::Dialog180Ringing).await {
                    error!("Failed to process Dialog180Ringing: {}", e);
                }
            } else if event_str.contains("Terminated") {
                if let Err(e) = self.state_machine.process_event(&sid, EventType::DialogBYE).await {
                    error!("Failed to process DialogBYE: {}", e);
                }
            }
        }
        Ok(())
    }
    
    async fn handle_call_terminated(&self, event_str: &str) -> Result<()> {
        info!("🎯 [handle_call_terminated] Called with event: {}",
              if event_str.len() > 200 { &event_str[..200] } else { event_str });

        if let Some(session_id_str) = self.extract_session_id(event_str) {
            let session_id = SessionId(session_id_str.clone());

            // Skip if this session isn't ours
            if self.state_machine.store.get_session(&session_id).await.is_err() {
                debug!("Ignoring CallTerminated for session {} - not in our store", session_id);
                return Ok(());
            }

            info!("🎯 [handle_call_terminated] Extracted session_id: {}", session_id_str);
            let reason = self.extract_field(event_str, "reason: ").unwrap_or_else(|| "Unknown".to_string());
            
            info!("🎯 [handle_call_terminated] Processing DialogTerminated for session {} with reason: {}", 
                  session_id, reason);
            
            // Process DialogTerminated to complete Terminating → Terminated transition
            // (DialogBYE was already processed when hangup was initiated)
            if let Err(e) = self.state_machine.process_event(
                &session_id,
                EventType::DialogTerminated
                ).await {
                        error!("Failed to process dialog terminated: {}", e);
                    } else {
                        info!("✅ [handle_call_terminated] DialogTerminated processed successfully for {}", session_id);
                    }
            
            // Publish CallEnded to the global coordinator's "session_to_app"
            // channel, then release the session from the store + registry.
            {
                info!("🔔 [handle_call_terminated] Publishing CallEnded for session {}", session_id);
                let api_event = crate::api::events::Event::CallEnded {
                    call_id: session_id.clone(),
                    reason: reason.clone(),
                };
                self.publish_and_release_session(api_event, session_id.clone()).await;
            }
        } else {
            warn!("⚠️ [handle_call_terminated] Failed to extract session_id, cannot forward CallEnded event");
        }
        
        info!("🏁 [handle_call_terminated] Completed");
        Ok(())
    }
    
    async fn handle_dialog_error(&self, event_str: &str) -> Result<()> {
        if let Some(session_id) = self.extract_session_id(event_str) {
            let sid = SessionId(session_id);
            if self.state_machine.store.get_session(&sid).await.is_err() {
                debug!("Ignoring DialogError for session {} - not in our store", sid);
                return Ok(());
            }
            let error = self.extract_field(event_str, "error: \"").unwrap_or_else(|| "Unknown error".to_string());
            if let Err(e) = self.state_machine.process_event(&sid, EventType::DialogError(error)).await {
                error!("Failed to process dialog error: {}", e);
            }
        }
        Ok(())
    }
    
    // Media event handlers
    async fn handle_media_stream_started(&self, event_str: &str) -> Result<()> {
        if let Some(session_id) = self.extract_session_id(event_str) {
            let sid = SessionId(session_id);
            if !self.is_our_session(&sid).await { return Ok(()); }
            if let Err(e) = self.state_machine.process_event(&sid, EventType::MediaSessionReady).await {
                error!("Failed to process media stream started: {}", e);
            }
        }
        Ok(())
    }

    async fn handle_media_stream_stopped(&self, event_str: &str) -> Result<()> {
        if let Some(session_id) = self.extract_session_id(event_str) {
            let sid = SessionId(session_id);
            if !self.is_our_session(&sid).await { return Ok(()); }
            let reason = self.extract_field(event_str, "reason: \"").unwrap_or_else(|| "Unknown reason".to_string());
            if let Err(e) = self.state_machine.process_event(&sid, EventType::MediaError(format!("Media stream stopped: {}", reason))).await {
                error!("Failed to process media stream stopped: {}", e);
            }
        }
        Ok(())
    }

    async fn handle_media_flow_established(&self, event_str: &str) -> Result<()> {
        if let Some(session_id) = self.extract_session_id(event_str) {
            let sid = SessionId(session_id);
            if !self.is_our_session(&sid).await { return Ok(()); }
            if let Err(e) = self.state_machine.process_event(&sid, EventType::MediaFlowEstablished).await {
                error!("Failed to process media flow established: {}", e);
            }
        }
        Ok(())
    }

    async fn handle_media_error(&self, event_str: &str) -> Result<()> {
        if let Some(session_id) = self.extract_session_id(event_str) {
            let sid = SessionId(session_id);
            if !self.is_our_session(&sid).await { return Ok(()); }
            let error = self.extract_field(event_str, "error: \"").unwrap_or_else(|| "Unknown error".to_string());
            if let Err(e) = self.state_machine.process_event(&sid, EventType::MediaError(error)).await {
                error!("Failed to process media error: {}", e);
            }
        }
        Ok(())
    }

    // New dialog event handlers
    async fn handle_dialog_state_changed(&self, event_str: &str) -> Result<()> {
        if let Some(session_id) = self.extract_session_id(event_str) {
            let sid = SessionId(session_id);
            if !self.is_our_session(&sid).await { return Ok(()); }
            let old_state = self.extract_field(event_str, "old_state: \"").unwrap_or_else(|| "unknown".to_string());
            let new_state = self.extract_field(event_str, "new_state: \"").unwrap_or_else(|| "unknown".to_string());
            if let Err(e) = self.state_machine.process_event(&sid, EventType::DialogStateChanged { old_state, new_state }).await {
                error!("Failed to process DialogStateChanged: {}", e);
            }
        }
        Ok(())
    }

    async fn handle_reinvite_received(&self, event_str: &str) -> Result<()> {
        if let Some(session_id) = self.extract_session_id(event_str) {
            let sid = SessionId(session_id);
            if !self.is_our_session(&sid).await { return Ok(()); }
            let sdp = self.extract_field(event_str, "sdp: Some(\"")
                .map(|s| s.replace("\\r\\n", "\r\n").replace("\\n", "\n").replace("\\\"", "\""));
            // `method` is an uppercase SIP method string emitted by
            // dialog-core's cross-crate conversion ("INVITE" or "UPDATE").
            // Default to re-INVITE for backward compat if the field is
            // missing — INVITE is the historic payload of this event.
            let method = self.extract_field(event_str, "method: \"")
                .unwrap_or_else(|| "INVITE".to_string());
            let event = if method.eq_ignore_ascii_case("UPDATE") {
                EventType::UpdateReceived { sdp }
            } else {
                EventType::ReinviteReceived { sdp }
            };
            if let Err(e) = self.state_machine.process_event(&sid, event).await {
                error!("Failed to process {} (method {}): {}",
                    "ReinviteReceived/UpdateReceived", method, e);
            }
        }
        Ok(())
    }
    
    async fn handle_transfer_requested(&self, event_str: &str) -> Result<()> {
        if let Some(session_id_str) = self.extract_session_id(event_str) {
            let session_id = SessionId(session_id_str.clone());

            // Skip if this session isn't ours
            if self.state_machine.store.get_session(&session_id).await.is_err() {
                debug!("Ignoring TransferRequested for session {} - not in our store", session_id);
                return Ok(());
            }

            let refer_to = self.extract_field(event_str, "refer_to: \"").unwrap_or_else(|| "unknown".to_string());
            let transfer_type = self.extract_field(event_str, "transfer_type: \"").unwrap_or_else(|| "blind".to_string());
            let transaction_id = self.extract_field(event_str, "transaction_id: \"").unwrap_or_else(|| "unknown".to_string());

            // RFC 3515 Compliance: Store transferor session ID
            if let Ok(mut session) = self.state_machine.store.get_session(&session_id).await {
                session.transferor_session_id = Some(session_id.clone());
                if let Err(e) = self.state_machine.store.update_session(session).await {
                    error!("Failed to store transferor session ID: {}", e);
                }
            }

            if let Err(e) = self.state_machine.process_event(
                &session_id,
                EventType::TransferRequested { 
                    refer_to: refer_to.clone(), 
                    transfer_type: transfer_type.clone(),
                    transaction_id: transaction_id.clone(),
                }
            ).await {
                error!("Failed to process TransferRequested: {}", e);
            }

            // Publish ReferReceived event to the global coordinator's "session_to_app" channel.
            {
                debug!("🔍 [DEBUG] Publishing ReferReceived event to global coordinator");
                let api_event = crate::api::events::Event::ReferReceived {
                    call_id: session_id.clone(),
                    refer_to: refer_to.clone(),
                    referred_by: None, // TODO: Extract from event if available
                    replaces: None,    // TODO: Extract from event if available
                    transaction_id: transaction_id.clone(),
                    transfer_type: transfer_type.clone(),
                };
                let wrapped = crate::adapters::SessionApiCrossCrateEvent::new(api_event);
                let coordinator = self.global_coordinator.clone();
                tokio::spawn(async move {
                    if let Err(e) = coordinator.publish(wrapped).await {
                        tracing::warn!("Failed to publish ReferReceived to global coordinator: {}", e);
                    }
                });
            }
        }
        Ok(())
    }

    async fn handle_ack_sent(&self, event_str: &str) -> Result<()> {
        // Extract dialog_id from the event
        let dialog_id_str = self.extract_field(event_str, "dialog_id: DialogId(")
            .or_else(|| self.extract_field(event_str, "dialog_id: \""))
            .unwrap_or_else(|| "unknown".to_string());

        // Parse the dialog ID to look up the session
        if let Ok(dialog_uuid) = uuid::Uuid::parse_str(&dialog_id_str.trim_end_matches(')')) {
            let rvoip_dialog_id = rvoip_dialog_core::DialogId(dialog_uuid);

            // Find the session ID from dialog ID
            if let Some(entry) = self.dialog_adapter.dialog_to_session.get(&rvoip_dialog_id) {
                let session_id = entry.value().clone();
                drop(entry);

                info!("ACK was sent by dialog-core for dialog {}, triggering DialogACK event for session {}", dialog_id_str, session_id);

                // Trigger DialogACK event in state machine
                // This allows UAS to transition from "Answering" -> "Active"
                if let Err(e) = self.state_machine.process_event(
                    &session_id,
                    EventType::DialogACK,
                ).await {
                    error!("Failed to process DialogACK event after AckSent: {}", e);
                }
            } else {
                warn!("Received AckSent for unknown dialog {}", dialog_id_str);
            }
        }

        Ok(())
    }

    async fn handle_ack_received(&self, event_str: &str) -> Result<()> {
        // Extract session_id directly from the cross-crate event
        let session_id_str = self.extract_session_id(event_str)
            .unwrap_or_else(|| {
                warn!("Could not extract session_id from AckReceived event");
                "unknown".to_string()
            });

        info!("📨 ACK was received by dialog-core, triggering DialogACK event for session {}", session_id_str);

        // Check if this session belongs to us — multiple peers share the global event bus
        let session_id = SessionId(session_id_str.clone());
        if self.state_machine.store.get_session(&session_id).await.is_err() {
            debug!("Ignoring AckReceived for session {} - not in our store", session_id_str);
            return Ok(());
        }

        info!("🔍 About to call process_event with DialogACK");

        // Trigger DialogACK event in state machine
        // This allows UAS to transition from "Answering" -> "Active"
        match self.state_machine.process_event(
            &SessionId(session_id_str.clone()),
            EventType::DialogACK,
        ).await {
            Ok(_) => {
                info!("✅ DialogACK processed successfully for session {}", session_id_str);
            }
            Err(e) => {
                error!("❌ Failed to process DialogACK event after AckReceived: {}", e);
            }
        }

        info!("🏁 Finished handle_ack_received for session {}", session_id_str);
        Ok(())
    }

    // New media event handlers
    async fn handle_media_quality_degraded(&self, event_str: &str) -> Result<()> {
        if let Some(session_id) = self.extract_session_id(event_str) {
            let sid = SessionId(session_id);
            if !self.is_our_session(&sid).await { return Ok(()); }
            let packet_loss_percent = self.extract_field(event_str, "packet_loss: ")
                .and_then(|s| s.parse::<f32>().ok())
                .map(|f| (f * 100.0) as u32)
                .unwrap_or(0);
            let jitter_ms = self.extract_field(event_str, "jitter: ")
                .and_then(|s| s.parse::<f32>().ok())
                .map(|f| (f * 1000.0) as u32)
                .unwrap_or(0);
            let severity = self.extract_field(event_str, "severity: \"").unwrap_or_else(|| "unknown".to_string());
            if let Err(e) = self.state_machine.process_event(&sid, EventType::MediaQualityDegraded { packet_loss_percent, jitter_ms, severity }).await {
                error!("Failed to process MediaQualityDegraded: {}", e);
            }
        }
        Ok(())
    }

    async fn handle_dtmf_detected(&self, event_str: &str) -> Result<()> {
        if let Some(session_id) = self.extract_session_id(event_str) {
            let sid = SessionId(session_id);
            if !self.is_our_session(&sid).await { return Ok(()); }
            let digit = self.extract_field(event_str, "digit: '")
                .and_then(|s| s.chars().next())
                .unwrap_or('?');
            let duration_ms = self.extract_field(event_str, "duration_ms: ")
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(0);
            if let Err(e) = self.state_machine.process_event(&sid, EventType::DtmfDetected { digit, duration_ms }).await {
                error!("Failed to process DtmfDetected: {}", e);
            }
        }
        Ok(())
    }

    async fn handle_rtp_timeout(&self, event_str: &str) -> Result<()> {
        if let Some(session_id) = self.extract_session_id(event_str) {
            let sid = SessionId(session_id);
            if !self.is_our_session(&sid).await { return Ok(()); }
            let last_packet_time = self.extract_field(event_str, "last_packet_time: \"").unwrap_or_else(|| "unknown".to_string());
            if let Err(e) = self.state_machine.process_event(&sid, EventType::RtpTimeout { last_packet_time }).await {
                error!("Failed to process RtpTimeout: {}", e);
            }
        }
        Ok(())
    }

    async fn handle_packet_loss_threshold_exceeded(&self, event_str: &str) -> Result<()> {
        if let Some(session_id) = self.extract_session_id(event_str) {
            let sid = SessionId(session_id);
            if !self.is_our_session(&sid).await { return Ok(()); }
            let loss_percentage = self.extract_field(event_str, "loss_percentage: ")
                .and_then(|s| s.parse::<f32>().ok())
                .map(|f| (f * 100.0) as u32)
                .unwrap_or(0);
            if let Err(e) = self.state_machine.process_event(&sid, EventType::PacketLossThresholdExceeded { loss_percentage }).await {
                error!("Failed to process PacketLossThresholdExceeded: {}", e);
            }
        }
        Ok(())
    }
}