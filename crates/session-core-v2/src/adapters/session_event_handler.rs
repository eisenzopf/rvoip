//! Session Event Handler - Central hub for ALL cross-crate event handling
//!
//! This is the ONLY place where cross-crate events are handled.
//! - Receives events from dialog-core and media-core
//! - Routes them to the state machine
//! - Publishes events to dialog-core and media-core
//!
//! NO OTHER MODULE should interact with the GlobalEventCoordinator directly.

use std::sync::Arc;
use std::str::FromStr;
use anyhow::Result;
use tokio::sync::mpsc;
use dashmap::DashMap;
use rvoip_infra_common::events::coordinator::{CrossCrateEventHandler, GlobalEventCoordinator};
use rvoip_infra_common::events::cross_crate::{
    CrossCrateEvent, RvoipCrossCrateEvent,
    DialogToSessionEvent, MediaToSessionEvent,
    SessionToDialogEvent, SessionToMediaEvent,
};
use crate::state_table::types::{SessionId, EventType, Role};
use crate::state_machine::StateMachine as StateMachineExecutor;
use crate::errors::{SessionError, Result as SessionResult};
use crate::adapters::{DialogAdapter, MediaAdapter};
use crate::session_registry::SessionRegistry;
use crate::types::DialogId;
use tracing::{debug, info, error, warn};

/// Handler for processing cross-crate events in session-core-v2
#[derive(Clone)]
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
            while let Some(event) = dialog_sub.recv().await {
                if let Err(e) = handler.handle(event).await {
                    error!("Error handling dialog-to-session event: {}", e);
                }
            }
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
                return Some(event_str[start..start+end].to_string());
            }
        }
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
            if let Some(session_id) = call_id.split('@').next() {
                // Only trigger state transition - all logic should be in the state machine
                if let Err(e) = self.state_machine.process_event(
                    &SessionId(session_id.to_string()),
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
        let dialog_id_str = self.extract_field(event_str, "session_id: \"").unwrap_or_else(|| "unknown".to_string());
        let call_id = self.extract_field(event_str, "call_id: \"").unwrap_or_else(|| "unknown".to_string());
        let from = self.extract_field(event_str, "from: \"").unwrap_or_else(|| "unknown".to_string());
        let to = self.extract_field(event_str, "to: \"").unwrap_or_else(|| "unknown".to_string());
        let sdp = self.extract_field(event_str, "sdp_offer: Some(\"")
            .map(|s| s.replace("\\r\\n", "\r\n").replace("\\n", "\n").replace("\\\"", "\""));
        let transaction_id = self.extract_field(event_str, "transaction_id: \"").unwrap_or_else(|| "unknown".to_string());
        let source_addr = self.extract_field(event_str, "source_addr: \"").unwrap_or_else(|| "127.0.0.1:5060".to_string());
        
        // CRITICAL: IncomingCall is a special case - we must create session here
        // because we don't have a session ID yet from dialog-core
        let session_id = SessionId::new();
        
        // Create session in store - this is the ONLY place we create sessions outside state machine
        self.state_machine.store.create_session(
            session_id.clone(),
            Role::UAS,
            true,
        ).await.map_err(|e| SessionError::InternalError(format!("Failed to create session: {}", e)))?;
        
        // Store transaction info for response sending - required before state machine runs
        let dialog_uuid = uuid::Uuid::parse_str(&dialog_id_str).unwrap_or_else(|_| uuid::Uuid::new_v4());
        let transaction_key = rvoip_dialog_core::transaction::TransactionKey::from_str(&transaction_id)
            .unwrap_or_else(|_| {
                // Create a minimal transaction key
                rvoip_dialog_core::transaction::TransactionKey::new(
                    transaction_id.clone(),
                    rvoip_sip_core::Method::Invite,
                    true // is_server = true for incoming calls
                )
            });
        let source_socket_addr = source_addr.parse::<std::net::SocketAddr>()
            .unwrap_or_else(|_| std::net::SocketAddr::from(([127, 0, 0, 1], 5061)));
        
        let dummy_request = rvoip_sip_core::Request::new(
            rvoip_sip_core::Method::Invite,
            rvoip_sip_core::Uri::from_str(&to).unwrap_or_else(|_| rvoip_sip_core::Uri::from_str("sip:unknown@unknown").unwrap())
        );
        
        self.dialog_adapter.incoming_requests.insert(
            session_id.clone(),
            (dummy_request, transaction_key, source_socket_addr)
        );
        
        // Store mapping info for state machine to use
        self.registry.map_dialog(session_id.clone(), DialogId(dialog_uuid));
        self.registry.store_pending_incoming_call(
            session_id.clone(),
            crate::types::IncomingCallInfo {
                session_id: session_id.clone(),
                from: from.clone(),
                to: to.clone(),
                        call_id: call_id.clone(),
                dialog_id: DialogId(dialog_uuid),
            }
        );
        
        // Process the event - state machine will handle the rest
        let event_type = EventType::IncomingCall { from: from.clone(), sdp };
        
        if let Err(e) = self.state_machine.process_event(
            &session_id,
            event_type
        ).await {
            error!("Failed to process incoming call event: {}", e);
            // Clean up on failure
            self.state_machine.store.remove_session(&session_id).await;
            self.registry.remove_session(&session_id);
        } else {
            // Notify about incoming call after successful processing
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
        // Extract session_id and optional SDP answer
        let session_id = self.extract_session_id(event_str).unwrap_or_else(|| "unknown".to_string());
        let sdp_answer = self.extract_field(event_str, "sdp_answer: Some(\"")
            .map(|s| s.replace("\\r\\n", "\r\n").replace("\\n", "\n").replace("\\\"", "\""));
        
        // Only trigger state transition - all logic should be in the state machine
        if let Err(e) = self.state_machine.process_event(
            &SessionId(session_id.clone()),
            EventType::CallEstablished { session_id: session_id.clone(), sdp_answer }
        ).await {
            error!("Failed to process CallEstablished event: {}", e);
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
        if let Some(session_id) = self.extract_session_id(event_str) {
            if let Err(e) = self.state_machine.process_event(
                &SessionId(session_id),
                EventType::DialogBYE
                ).await {
                        error!("Failed to process call termination: {}", e);
                    }
                }
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
        if let Some(session_id) = self.extract_session_id(event_str) {
            let refer_to = self.extract_field(event_str, "refer_to: \"").unwrap_or_else(|| "unknown".to_string());
            let transfer_type = self.extract_field(event_str, "transfer_type: \"").unwrap_or_else(|| "blind".to_string());
            
            if let Err(e) = self.state_machine.process_event(
                &SessionId(session_id),
                EventType::TransferRequested { refer_to, transfer_type }
            ).await {
                error!("Failed to process TransferRequested: {}", e);
            }
        }
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