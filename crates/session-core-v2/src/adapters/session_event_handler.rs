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
use dashmap::DashMap;
use infra_common::events::coordinator::{CrossCrateEventHandler, GlobalEventCoordinator};
use infra_common::events::cross_crate::{
    CrossCrateEvent, RvoipCrossCrateEvent,
    DialogToSessionEvent, MediaToSessionEvent,
    SessionToDialogEvent, SessionToMediaEvent,
};
use crate::state_table::types::{SessionId, EventType};
use crate::state_machine::StateMachine as StateMachineExecutor;
use crate::errors::{SessionError, Result as SessionResult};
use crate::adapters::{DialogAdapter, MediaAdapter};
use crate::session_registry::SessionRegistry;
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
        }
    }
    
    /// Start event processing loops
    pub async fn start(&self) -> SessionResult<()> {
        // Set up backward compatibility channels for dialog-core
        self.setup_dialog_channels().await?;
        
        // Set up backward compatibility channels for media-core
        self.setup_media_channels().await?;
        
        // Start subscription to global events
        self.start_global_event_subscriptions().await?;
        
        Ok(())
    }
    
    /// Set up channels for dialog-core backward compatibility
    async fn setup_dialog_channels(&self) -> SessionResult<()> {
        use rvoip_dialog_core::events::{SessionCoordinationEvent, DialogEvent};
        
        // Create channels for dialog events
        let (session_tx, mut session_rx) = mpsc::channel(1000);
        let (dialog_tx, mut dialog_rx) = mpsc::channel(1000);
        
        // Set channels on dialog API
        self.dialog_adapter.dialog_api
            .set_session_coordinator(session_tx)
            .await
            .map_err(|e| SessionError::DialogError(format!("Failed to set session coordinator: {}", e)))?;
            
        self.dialog_adapter.dialog_api
            .set_dialog_event_sender(dialog_tx)
            .await
            .map_err(|e| SessionError::DialogError(format!("Failed to set dialog event sender: {}", e)))?;
        
        // Spawn task to process session coordination events
        let handler = self.clone();
        tokio::spawn(async move {
            while let Some(event) = session_rx.recv().await {
                if let Err(e) = handler.handle_session_coordination_event(event).await {
                    error!("Error handling session coordination event: {}", e);
                }
            }
        });
        
        // Spawn task to process dialog events
        let handler = self.clone();
        tokio::spawn(async move {
            while let Some(event) = dialog_rx.recv().await {
                if let Err(e) = handler.handle_dialog_event(event).await {
                    error!("Error handling dialog event: {}", e);
                }
            }
        });
        
        Ok(())
    }
    
    /// Set up channels for media-core backward compatibility
    async fn setup_media_channels(&self) -> SessionResult<()> {
        use rvoip_media_core::relay::controller::MediaSessionEvent;
        
        // Get event receiver from media controller
        let mut event_rx = self.media_adapter.controller.take_event_receiver()
            .await
            .ok_or_else(|| SessionError::InternalError("Failed to get media event receiver".into()))?;
        
        // Spawn task to process media events
        let handler = self.clone();
        tokio::spawn(async move {
            while let Some(event) = event_rx.recv().await {
                if let Err(e) = handler.handle_media_event(event).await {
                    error!("Error handling media event: {}", e);
                }
            }
        });
        
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
        
        // Since we can't directly downcast Arc<dyn CrossCrateEvent>, we'll use the
        // event_type() to determine what kind of event it is and parse accordingly.
        // This is a workaround until we have proper downcast support.
        
        // Try to extract the event data from the debug representation
        let event_str = format!("{:?}", event);
        
        match event.event_type() {
            "dialog_to_session" => {
                info!("Processing dialog-to-session event: {}", event_str);
                
                // Parse the event to extract the session ID and event type
                if let Some(session_id) = self.extract_session_id(&event_str) {
                    if let Some(event_type) = self.convert_dialog_event(&event_str) {
                        debug!("Converted dialog event to state machine event: {:?}", event_type);
                        
                        // Process the event through the state machine
                        if let Err(e) = self.state_machine.process_event(
                            &SessionId(session_id),
                            event_type
                        ).await {
                            error!("Failed to process dialog event: {}", e);
                        }
                    }
                }
            }
            "media_to_session" => {
                info!("Processing media-to-session event: {}", event_str);
                
                // Parse the event to extract the session ID and event type
                if let Some(session_id) = self.extract_session_id(&event_str) {
                    if let Some(event_type) = self.convert_media_event(&event_str) {
                        debug!("Converted media event to state machine event: {:?}", event_type);
                        
                        // Process the event through the state machine
                        if let Err(e) = self.state_machine.process_event(
                            &SessionId(session_id),
                            event_type
                        ).await {
                            error!("Failed to process media event: {}", e);
                        }
                    }
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
    /// Handle session coordination events from dialog-core (backward compatibility)
    async fn handle_session_coordination_event(&self, event: rvoip_dialog_core::events::SessionCoordinationEvent) -> SessionResult<()> {
        use rvoip_dialog_core::events::SessionCoordinationEvent;
        use rvoip_sip_core::{Request, Response, StatusCode};
        
        match event {
            SessionCoordinationEvent::IncomingCall { dialog_id, transaction_id, request, source } => {
                // Extract Call-ID for correlation
                let call_id = request.call_id()
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| format!("unknown-{}", uuid::Uuid::new_v4()));
                let session_id = SessionId::new();
                
                // Store mappings in adapters (they still maintain the mappings)
                self.dialog_adapter.dialog_to_session.insert(dialog_id.clone(), session_id.clone());
                self.dialog_adapter.session_to_dialog.insert(session_id.clone(), dialog_id);
                self.dialog_adapter.callid_to_session.insert(call_id.clone(), session_id.clone());
                self.dialog_adapter.incoming_requests.insert(session_id.clone(), (request.clone(), transaction_id, source));
                
                // Extract SDP if present
                let sdp = if !request.body().is_empty() {
                    String::from_utf8(request.body().to_vec()).ok()
                } else {
                    None
                };
                
                // Convert to state machine event
                let event_type = EventType::IncomingCall {
                    from: request.from()
                        .map(|f| f.to_string())
                        .unwrap_or_else(|| "anonymous".to_string()),
                    sdp: sdp.clone(),
                };
                
                // Process through state machine
                if let Err(e) = self.state_machine.process_event(&session_id, event_type).await {
                    error!("Failed to process incoming call: {}", e);
                }
                
                // Also publish as cross-crate event
                let cross_crate_event = RvoipCrossCrateEvent::DialogToSession(
                    DialogToSessionEvent::IncomingCall {
                        session_id: session_id.0.clone(),
                        call_id: call_id.clone(),
                        from: request.from()
                            .map(|f| f.to_string())
                            .unwrap_or_else(|| "anonymous".to_string()),
                        to: request.to()
                            .map(|t| t.to_string())
                            .unwrap_or_else(|| "unknown".to_string()),
                        sdp_offer: sdp,
                        headers: std::collections::HashMap::new(),
                    }
                );
                if let Err(e) = self.global_coordinator.publish(Arc::new(cross_crate_event)).await {
                    error!("Failed to publish IncomingCall event: {}", e);
                }
            }
            
            SessionCoordinationEvent::ResponseReceived { dialog_id, response, .. } => {
                if let Some(session_id) = self.dialog_adapter.dialog_to_session.get(&dialog_id) {
                    let session_id = session_id.clone();
                    let status_code = response.status_code();
                    
                    // Store 200 OK for ACK
                    if status_code == 200 {
                        if let Ok(mut session) = self.state_machine.store.get_session(&session_id).await {
                            if let Ok(serialized) = bincode::serialize(&response) {
                                session.last_200_ok = Some(serialized);
                                if !response.body().is_empty() {
                                    if let Some(sdp) = String::from_utf8(response.body().to_vec()).ok() {
                                        session.remote_sdp = Some(sdp);
                                    }
                                }
                                let _ = self.state_machine.store.update_session(session).await;
                            }
                        }
                    }
                    
                    // Convert to event type
                    let event_type = match status_code {
                        100 => return Ok(()), // Ignore 100 Trying
                        180 => EventType::Dialog180Ringing,
                        200 => EventType::Dialog200OK,
                        code if code >= 400 => EventType::DialogError(format!("Call failed: {}", code)),
                        _ => return Ok(()),
                    };
                    
                    // Process through state machine
                    if let Err(e) = self.state_machine.process_event(&session_id, event_type).await {
                        error!("Failed to process dialog response: {}", e);
                    }
                }
            }
            
            SessionCoordinationEvent::CallTerminating { dialog_id, reason } => {
                if let Some(session_id) = self.dialog_adapter.dialog_to_session.get(&dialog_id) {
                    let session_id = session_id.clone();
                    
                    // Process through state machine
                    if let Err(e) = self.state_machine.process_event(&session_id, EventType::DialogBYE).await {
                        error!("Failed to process call termination: {}", e);
                    }
                }
            }
            
            _ => {
                // Ignore other events
            }
        }
        
        Ok(())
    }
    
    /// Handle dialog events from dialog-core (backward compatibility)
    async fn handle_dialog_event(&self, event: rvoip_dialog_core::events::DialogEvent) -> SessionResult<()> {
        use rvoip_dialog_core::events::DialogEvent;
        
        match event {
            DialogEvent::Created { dialog_id } => {
                debug!("Dialog created: {:?}", dialog_id);
            }
            
            DialogEvent::StateChanged { dialog_id, old_state, new_state } => {
                debug!("Dialog state changed: {:?} from {:?} to {:?}", dialog_id, old_state, new_state);
            }
            
            DialogEvent::Terminated { dialog_id, reason } => {
                if let Some(session_id) = self.dialog_adapter.dialog_to_session.get(&dialog_id) {
                    debug!("Dialog terminated: {:?}, reason: {}", dialog_id, reason);
                }
            }
            
            _ => {
                // Ignore other events
            }
        }
        
        Ok(())
    }
    
    /// Handle media events from media-core (backward compatibility)
    async fn handle_media_event(&self, event: rvoip_media_core::relay::controller::MediaSessionEvent) -> SessionResult<()> {
        use rvoip_media_core::relay::controller::MediaSessionEvent;
        
        match event {
            MediaSessionEvent::SessionCreated { dialog_id, .. } => {
                if let Some(session_id) = self.media_adapter.dialog_to_session.get(&dialog_id) {
                    debug!("Media session created for {}", session_id.0);
                    
                    // Process through state machine
                    if let Err(e) = self.state_machine.process_event(&session_id, EventType::MediaSessionReady).await {
                        error!("Failed to process media session created: {}", e);
                    }
                }
            }
            
            MediaSessionEvent::SessionDestroyed { dialog_id, .. } => {
                if let Some(session_id) = self.media_adapter.dialog_to_session.get(&dialog_id) {
                    debug!("Media session destroyed for {}", session_id.0);
                }
            }
            
            MediaSessionEvent::SessionFailed { dialog_id, error } => {
                if let Some(session_id) = self.media_adapter.dialog_to_session.get(&dialog_id) {
                    let session_id = session_id.clone();
                    
                    // Process through state machine
                    let event_type = EventType::MediaError(error.clone());
                    if let Err(e) = self.state_machine.process_event(&session_id, event_type).await {
                        error!("Failed to process media error: {}", e);
                    }
                    
                    // Also publish as cross-crate event
                    let cross_crate_event = RvoipCrossCrateEvent::MediaToSession(
                        MediaToSessionEvent::MediaError {
                            session_id: session_id.0.clone(),
                            error: error.clone(),
                            error_code: None,
                        }
                    );
                    if let Err(e) = self.global_coordinator.publish(Arc::new(cross_crate_event)).await {
                        error!("Failed to publish MediaError: {}", e);
                    }
                }
            }
            
            MediaSessionEvent::RemoteAddressUpdated { dialog_id, remote_addr } => {
                if let Some(session_id) = self.media_adapter.dialog_to_session.get(&dialog_id) {
                    debug!("Remote address updated for {}: {}", session_id.0, remote_addr);
                }
            }
            
            _ => {
                // Ignore other events
            }
        }
        
        Ok(())
    }
    
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
    
    /// Convert dialog event string to EventType (temporary workaround)
    fn convert_dialog_event(&self, event_str: &str) -> Option<EventType> {
        if event_str.contains("IncomingCall") {
            // Extract from field
            let from = if let Some(start) = event_str.find("from: \"") {
                let start = start + 7;
                if let Some(end) = event_str[start..].find('"') {
                    event_str[start..start+end].to_string()
                } else {
                    "unknown".to_string()
                }
            } else {
                "unknown".to_string()
            };
            
            Some(EventType::IncomingCall { from, sdp: None })
        } else if event_str.contains("CallEstablished") {
            Some(EventType::Dialog200OK)
        } else if event_str.contains("CallStateChanged") {
            if event_str.contains("Ringing") {
                Some(EventType::Dialog180Ringing)
            } else if event_str.contains("Active") {
                Some(EventType::Dialog200OK)
            } else if event_str.contains("Terminated") {
                Some(EventType::DialogBYE)
            } else {
                None
            }
        } else if event_str.contains("CallTerminated") {
            Some(EventType::DialogBYE)
        } else if event_str.contains("CallRejected") {
            Some(EventType::Dialog4xxFailure(486))
        } else {
            None
        }
    }
    
    /// Convert media event string to EventType (temporary workaround)
    fn convert_media_event(&self, event_str: &str) -> Option<EventType> {
        if event_str.contains("MediaStreamStarted") {
            Some(EventType::MediaSessionReady)
        } else if event_str.contains("MediaFlowEstablished") {
            Some(EventType::MediaFlowEstablished)
        } else if event_str.contains("MediaStreamStopped") || event_str.contains("MediaError") {
            Some(EventType::MediaError("Media stream error".to_string()))
        } else {
            None
        }
    }
}