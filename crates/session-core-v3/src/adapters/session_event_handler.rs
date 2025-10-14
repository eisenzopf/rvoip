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

/// Handler for processing cross-crate events in session-core-v2
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

    /// SimplePeer event channel for forwarding events
    simple_peer_event_tx: Option<tokio::sync::mpsc::Sender<crate::api::events::Event>>,
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
            simple_peer_event_tx: None,
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
            simple_peer_event_tx: None,
        }
    }

    /// Create event handler with SimplePeer event integration
    pub fn with_simple_peer_events(
        state_machine: Arc<StateMachineExecutor>,
        global_coordinator: Arc<GlobalEventCoordinator>,
        dialog_adapter: Arc<DialogAdapter>,
        media_adapter: Arc<MediaAdapter>,
        registry: Arc<SessionRegistry>,
        incoming_call_tx: mpsc::Sender<crate::types::IncomingCallInfo>,
        simple_peer_event_tx: tokio::sync::mpsc::Sender<crate::api::events::Event>,
    ) -> Self {
        Self {
            state_machine,
            global_coordinator,
            dialog_adapter,
            media_adapter,
            registry,
            incoming_call_tx: Some(incoming_call_tx),
            simple_peer_event_tx: Some(simple_peer_event_tx),
        }
    }
    
    /// Start event processing loops
    pub async fn start(&self) -> SessionResult<()> {
        // Start subscription to global events
        self.start_global_event_subscriptions().await?;
        
        Ok(())
    }
    
    
    
    /// Start subscriptions to global cross-crate events
    async fn start_global_event_subscriptions(&self) -> SessionResult<()> {
        // Subscribe to dialog-to-session events
        let mut dialog_sub = self.global_coordinator
            .subscribe("dialog_to_session")
            .await
            .map_err(|e| SessionError::InternalError(format!("Failed to subscribe to dialog events: {}", e)))?;
            
        let handler = self.clone();
        tokio::spawn(async move {
            info!("🔔 [session_event_handler] Started dialog-to-session event loop");
            while let Some(event) = dialog_sub.recv().await {
                info!("🔔 [session_event_handler] Received event from channel: {:?}", event);
                if let Err(e) = handler.handle(event).await {
                    error!("Error handling dialog-to-session event: {}", e);
                }
            }
            warn!("🔔 [session_event_handler] Dialog-to-session event loop ended");
        });
        
        // Subscribe to media-to-session events
        let mut media_sub = self.global_coordinator
            .subscribe("media_to_session")
            .await
            .map_err(|e| SessionError::InternalError(format!("Failed to subscribe to media events: {}", e)))?;
            
        let handler = self.clone();
        tokio::spawn(async move {
            while let Some(event) = media_sub.recv().await {
                if let Err(e) = handler.handle(event).await {
                    error!("Error handling media-to-session event: {}", e);
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

        // IMPORTANT: Check if this event is for OUR dialog instance
        // Parse dialog UUID to check against our dialog adapter
        if let Ok(dialog_uuid) = uuid::Uuid::parse_str(&dialog_id_str) {
            let rvoip_dialog_id = rvoip_dialog_core::DialogId(dialog_uuid);

            // Check if this dialog exists in our dialog adapter's session_to_dialog map
            // If the dialog is already mapped, it means another peer is handling it
            if self.dialog_adapter.dialog_to_session.contains_key(&rvoip_dialog_id) {
                // This dialog is already being handled (probably by another peer in the same process)
                debug!("Ignoring IncomingCall for dialog {} - already handled by another peer", dialog_id_str);
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
            // Forward to SimplePeer event system
            if let Some(ref event_tx) = self.simple_peer_event_tx {
                debug!("🔍 [DEBUG] Forwarding IncomingCall event to SimplePeer");
                let event = crate::api::events::Event::IncomingCall {
                    call_id: session_id.clone(),
                    from: from.clone(),
                    to: to.clone(),
                    sdp: session_remote_sdp,
                };
                
                // Use try_send to avoid blocking if receiver stopped draining
                match event_tx.try_send(event) {
                    Ok(_) => debug!("🔍 [DEBUG] Successfully sent IncomingCall event to SimplePeer"),
                    Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                        warn!("⚠️ Event channel full, dropping IncomingCall event (receiver not draining)");
                    }
                    Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                        debug!("Event channel closed, receiver already shut down");
                    }
                }
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

        // Forward to SimplePeer event system
        if let Some(ref event_tx) = self.simple_peer_event_tx {
            debug!("🔍 [DEBUG] Forwarding CallAnswered event to SimplePeer");
            let event = crate::api::events::Event::CallAnswered {
                call_id: session_id.clone(),
                sdp: sdp_answer,
            };
            
            // Use try_send to avoid blocking if receiver stopped draining
            match event_tx.try_send(event) {
                Ok(_) => debug!("🔍 [DEBUG] Successfully sent CallAnswered event to SimplePeer"),
                Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                    warn!("⚠️ Event channel full, dropping CallAnswered event (receiver not draining)");
                }
                Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                    debug!("Event channel closed, receiver already shut down");
                }
            }
        }

        Ok(())
    }
    
    async fn handle_call_state_changed(&self, event_str: &str) -> Result<()> {
        if let Some(session_id) = self.extract_session_id(event_str) {
            if event_str.contains("Ringing") {
                        if let Err(e) = self.state_machine.process_event(
                            &SessionId(session_id),
                    EventType::Dialog180Ringing
                        ).await {
                    error!("Failed to process Dialog180Ringing: {}", e);
                }
            } else if event_str.contains("Terminated") {
                        if let Err(e) = self.state_machine.process_event(
                            &SessionId(session_id),
                    EventType::DialogBYE
                        ).await {
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
            info!("🎯 [handle_call_terminated] Extracted session_id: {}", session_id_str);
            
            let session_id = SessionId(session_id_str.clone());
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
            
            // Forward to SimplePeer event system
            if let Some(ref event_tx) = self.simple_peer_event_tx {
                info!("🔔 [handle_call_terminated] Forwarding CallEnded event to SimplePeer for session {}", session_id);
                let event = crate::api::events::Event::CallEnded {
                    call_id: session_id.clone(),
                    reason: reason.clone(),
                };
                
                // Use try_send to avoid blocking if the receiver stopped draining events
                // (e.g., if user code doesn't have an event loop after hangup)
                match event_tx.try_send(event) {
                    Ok(_) => {
                        info!("✅ Successfully sent CallEnded event to SimplePeer for session {}", session_id);
                    }
                    Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                        warn!("⚠️ Event channel full, dropping CallEnded event for session {} (receiver not draining)", session_id);
                    }
                    Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                        debug!("Event channel closed, receiver already shut down for session {}", session_id);
                    }
                }
            } else {
                warn!("⚠️ [handle_call_terminated] simple_peer_event_tx is None, cannot forward CallEnded for session {}", session_id);
            }
        } else {
            warn!("⚠️ [handle_call_terminated] Failed to extract session_id, cannot forward CallEnded event");
        }
        
        info!("🏁 [handle_call_terminated] Completed");
        Ok(())
    }
    
    async fn handle_dialog_error(&self, event_str: &str) -> Result<()> {
        if let Some(session_id) = self.extract_session_id(event_str) {
            let error = self.extract_field(event_str, "error: \"").unwrap_or_else(|| "Unknown error".to_string());
            
            if let Err(e) = self.state_machine.process_event(
                &SessionId(session_id),
                EventType::DialogError(error)
            ).await {
                error!("Failed to process dialog error: {}", e);
            }
        }
        Ok(())
    }
    
    // Media event handlers
    async fn handle_media_stream_started(&self, event_str: &str) -> Result<()> {
        if let Some(session_id) = self.extract_session_id(event_str) {
            if let Err(e) = self.state_machine.process_event(
                &SessionId(session_id),
                EventType::MediaSessionReady
            ).await {
                error!("Failed to process media stream started: {}", e);
            }
        }
        Ok(())
    }
    
    async fn handle_media_stream_stopped(&self, event_str: &str) -> Result<()> {
        if let Some(session_id) = self.extract_session_id(event_str) {
            let reason = self.extract_field(event_str, "reason: \"").unwrap_or_else(|| "Unknown reason".to_string());
            
            if let Err(e) = self.state_machine.process_event(
                &SessionId(session_id),
                EventType::MediaError(format!("Media stream stopped: {}", reason))
            ).await {
                error!("Failed to process media stream stopped: {}", e);
            }
        }
        Ok(())
    }
    
    async fn handle_media_flow_established(&self, event_str: &str) -> Result<()> {
        if let Some(session_id) = self.extract_session_id(event_str) {
            if let Err(e) = self.state_machine.process_event(
                &SessionId(session_id),
                EventType::MediaFlowEstablished
            ).await {
                error!("Failed to process media flow established: {}", e);
            }
        }
        Ok(())
    }
    
    async fn handle_media_error(&self, event_str: &str) -> Result<()> {
        if let Some(session_id) = self.extract_session_id(event_str) {
            let error = self.extract_field(event_str, "error: \"").unwrap_or_else(|| "Unknown error".to_string());
            
            if let Err(e) = self.state_machine.process_event(
                &SessionId(session_id),
                EventType::MediaError(error)
            ).await {
                error!("Failed to process media error: {}", e);
            }
        }
        Ok(())
    }
    
    // New dialog event handlers
    async fn handle_dialog_state_changed(&self, event_str: &str) -> Result<()> {
        if let Some(session_id) = self.extract_session_id(event_str) {
            let old_state = self.extract_field(event_str, "old_state: \"").unwrap_or_else(|| "unknown".to_string());
            let new_state = self.extract_field(event_str, "new_state: \"").unwrap_or_else(|| "unknown".to_string());
            
            if let Err(e) = self.state_machine.process_event(
                &SessionId(session_id),
                EventType::DialogStateChanged { old_state, new_state }
            ).await {
                error!("Failed to process DialogStateChanged: {}", e);
            }
        }
        Ok(())
    }
    
    async fn handle_reinvite_received(&self, event_str: &str) -> Result<()> {
        if let Some(session_id) = self.extract_session_id(event_str) {
            let sdp = self.extract_field(event_str, "sdp: Some(\"")
                .map(|s| s.replace("\\r\\n", "\r\n").replace("\\n", "\n").replace("\\\"", "\""));
            
            if let Err(e) = self.state_machine.process_event(
                &SessionId(session_id),
                EventType::ReinviteReceived { sdp }
            ).await {
                error!("Failed to process ReinviteReceived: {}", e);
            }
        }
        Ok(())
    }
    
    async fn handle_transfer_requested(&self, event_str: &str) -> Result<()> {
        if let Some(session_id_str) = self.extract_session_id(event_str) {
            let refer_to = self.extract_field(event_str, "refer_to: \"").unwrap_or_else(|| "unknown".to_string());
            let transfer_type = self.extract_field(event_str, "transfer_type: \"").unwrap_or_else(|| "blind".to_string());
            let transaction_id = self.extract_field(event_str, "transaction_id: \"").unwrap_or_else(|| "unknown".to_string());

            let session_id = SessionId(session_id_str.clone());

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

            // Forward to SimplePeer event system
            if let Some(ref event_tx) = self.simple_peer_event_tx {
                debug!("🔍 [DEBUG] Forwarding ReferReceived event to SimplePeer with transaction_id and transfer_type");
                let event = crate::api::events::Event::ReferReceived {
                    call_id: session_id.clone(),
                    refer_to: refer_to.clone(),
                    referred_by: None, // TODO: Extract from event if available
                    replaces: None,    // TODO: Extract from event if available
                    transaction_id: transaction_id.clone(),  // NEW: Include for NOTIFY correlation
                    transfer_type: transfer_type.clone(),     // NEW: Include transfer type
                };
                
                // Use try_send to avoid blocking if receiver stopped draining
                match event_tx.try_send(event) {
                    Ok(_) => debug!("🔍 [DEBUG] Successfully sent ReferReceived event to SimplePeer"),
                    Err(tokio::sync::mpsc::error::TrySendError::Full(_)) => {
                        warn!("⚠️ Event channel full, dropping ReferReceived event (receiver not draining)");
                    }
                    Err(tokio::sync::mpsc::error::TrySendError::Closed(_)) => {
                        debug!("Event channel closed, receiver already shut down");
                    }
                }
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
            let packet_loss_percent = self.extract_field(event_str, "packet_loss: ")
                .and_then(|s| s.parse::<f32>().ok())
                .map(|f| (f * 100.0) as u32)
                .unwrap_or(0);
            let jitter_ms = self.extract_field(event_str, "jitter: ")
                .and_then(|s| s.parse::<f32>().ok())
                .map(|f| (f * 1000.0) as u32)
                .unwrap_or(0);
            let severity = self.extract_field(event_str, "severity: \"").unwrap_or_else(|| "unknown".to_string());
            
            if let Err(e) = self.state_machine.process_event(
                &SessionId(session_id),
                EventType::MediaQualityDegraded { packet_loss_percent, jitter_ms, severity }
            ).await {
                error!("Failed to process MediaQualityDegraded: {}", e);
            }
        }
        Ok(())
    }
    
    async fn handle_dtmf_detected(&self, event_str: &str) -> Result<()> {
        if let Some(session_id) = self.extract_session_id(event_str) {
            let digit = self.extract_field(event_str, "digit: '")
                .and_then(|s| s.chars().next())
                .unwrap_or('?');
            let duration_ms = self.extract_field(event_str, "duration_ms: ")
                .and_then(|s| s.parse::<u32>().ok())
                .unwrap_or(0);
            
            if let Err(e) = self.state_machine.process_event(
                &SessionId(session_id),
                EventType::DtmfDetected { digit, duration_ms }
            ).await {
                error!("Failed to process DtmfDetected: {}", e);
            }
        }
        Ok(())
    }
    
    async fn handle_rtp_timeout(&self, event_str: &str) -> Result<()> {
        if let Some(session_id) = self.extract_session_id(event_str) {
            let last_packet_time = self.extract_field(event_str, "last_packet_time: \"").unwrap_or_else(|| "unknown".to_string());
            
            if let Err(e) = self.state_machine.process_event(
                &SessionId(session_id),
                EventType::RtpTimeout { last_packet_time }
            ).await {
                error!("Failed to process RtpTimeout: {}", e);
            }
        }
        Ok(())
    }
    
    async fn handle_packet_loss_threshold_exceeded(&self, event_str: &str) -> Result<()> {
        if let Some(session_id) = self.extract_session_id(event_str) {
            let loss_percentage = self.extract_field(event_str, "loss_percentage: ")
                .and_then(|s| s.parse::<f32>().ok())
                .map(|f| (f * 100.0) as u32)
                .unwrap_or(0);
            
            if let Err(e) = self.state_machine.process_event(
                &SessionId(session_id),
                EventType::PacketLossThresholdExceeded { loss_percentage }
            ).await {
                error!("Failed to process PacketLossThresholdExceeded: {}", e);
            }
        }
        Ok(())
    }
}