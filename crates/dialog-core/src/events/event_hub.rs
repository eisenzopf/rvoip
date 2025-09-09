//! Dialog Event Hub for Global Event Coordination
//!
//! This module provides the central event hub that integrates dialog-core with the global
//! event coordinator from infra-common, replacing channel-based communication.

use std::sync::Arc;
use anyhow::Result;
use async_trait::async_trait;
use tracing::{debug, info, warn, error};

use infra_common::events::coordinator::{GlobalEventCoordinator, CrossCrateEventHandler};
use infra_common::events::cross_crate::{
    CrossCrateEvent, RvoipCrossCrateEvent, DialogToSessionEvent, SessionToDialogEvent,
    DialogToTransportEvent, TransportToDialogEvent, CallState, TerminationReason
};

use crate::events::{DialogEvent, SessionCoordinationEvent};
use crate::dialog::{DialogId, DialogState};
use crate::errors::DialogError;
use crate::manager::DialogManager;

/// Dialog Event Hub that handles all cross-crate event communication
#[derive(Clone)]
pub struct DialogEventHub {
    /// Global event coordinator for cross-crate communication
    global_coordinator: Arc<GlobalEventCoordinator>,
    
    /// Reference to dialog manager for handling incoming events
    dialog_manager: Arc<DialogManager>,
}

impl std::fmt::Debug for DialogEventHub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("DialogEventHub")
            .field("global_coordinator", &"Arc<GlobalEventCoordinator>")
            .field("dialog_manager", &"Arc<DialogManager>")
            .finish()
    }
}

impl DialogEventHub {
    /// Create a new dialog event hub
    pub async fn new(
        global_coordinator: Arc<GlobalEventCoordinator>,
        dialog_manager: Arc<DialogManager>,
    ) -> Result<Arc<Self>> {
        let hub = Arc::new(Self {
            global_coordinator: global_coordinator.clone(),
            dialog_manager,
        });
        
        // Clone hub for registration (CrossCrateEventHandler must be implemented for DialogEventHub not Arc<DialogEventHub>)
        let handler = DialogEventHub {
            global_coordinator: global_coordinator.clone(),
            dialog_manager: hub.dialog_manager.clone(),
        };
        
        // Register as handler for session-to-dialog events
        global_coordinator
            .register_handler("session_to_dialog", handler.clone())
            .await?;
            
        // Register as handler for transport-to-dialog events
        global_coordinator
            .register_handler("transport_to_dialog", handler)
            .await?;
        
        info!("Dialog Event Hub initialized and registered with GlobalEventCoordinator");
        
        Ok(hub)
    }
    
    /// Publish a dialog event to the global bus
    pub async fn publish_dialog_event(&self, event: DialogEvent) -> Result<()> {
        debug!("Publishing dialog event: {:?}", event);
        
        // Convert to cross-crate event if applicable
        if let Some(cross_crate_event) = self.convert_dialog_to_cross_crate(event) {
            self.global_coordinator.publish(Arc::new(cross_crate_event)).await?;
        }
        
        Ok(())
    }
    
    /// Publish a session coordination event to the global bus
    pub async fn publish_session_coordination_event(&self, event: SessionCoordinationEvent) -> Result<()> {
        debug!("Publishing session coordination event: {:?}", event);
        
        // Convert to cross-crate event
        if let Some(cross_crate_event) = self.convert_coordination_to_cross_crate(event) {
            self.global_coordinator.publish(Arc::new(cross_crate_event)).await?;
        }
        
        Ok(())
    }
    
    /// Convert DialogEvent to cross-crate event
    fn convert_dialog_to_cross_crate(&self, event: DialogEvent) -> Option<RvoipCrossCrateEvent> {
        match event {
            DialogEvent::StateChanged { dialog_id, old_state, new_state } => {
                // Map dialog states to cross-crate call states
                let call_state = match new_state {
                    DialogState::Initial => CallState::Initiating,
                    DialogState::Early => CallState::Ringing,
                    DialogState::Confirmed => CallState::Active,
                    DialogState::Recovering => CallState::Active, // Still active but recovering
                    DialogState::Terminated => CallState::Terminated,
                };
                
                // Get session ID from dialog ID mapping
                if let Some(session_id) = self.dialog_manager.get_session_id(&dialog_id) {
                    Some(RvoipCrossCrateEvent::DialogToSession(
                        DialogToSessionEvent::CallStateChanged {
                            session_id,
                            new_state: call_state,
                            reason: None,
                        }
                    ))
                } else {
                    warn!("No session ID found for dialog {:?}", dialog_id);
                    None
                }
            }
            
            DialogEvent::Terminated { dialog_id, reason } => {
                if let Some(session_id) = self.dialog_manager.get_session_id(&dialog_id) {
                    Some(RvoipCrossCrateEvent::DialogToSession(
                        DialogToSessionEvent::CallTerminated {
                            session_id,
                            reason: TerminationReason::RemoteHangup,
                        }
                    ))
                } else {
                    None
                }
            }
            
            _ => None, // Other events are internal only
        }
    }
    
    /// Convert SessionCoordinationEvent to cross-crate event
    fn convert_coordination_to_cross_crate(&self, event: SessionCoordinationEvent) -> Option<RvoipCrossCrateEvent> {
        match event {
            SessionCoordinationEvent::IncomingCall { dialog_id, transaction_id, request, source } => {
                let call_id = request.call_id()
                    .map(|c| c.to_string())
                    .unwrap_or_else(|| format!("unknown-{}", uuid::Uuid::new_v4()));
                
                let from = request.from()
                    .map(|f| f.to_string())
                    .unwrap_or_else(|| "anonymous".to_string());
                    
                let to = request.to()
                    .map(|t| t.to_string())
                    .unwrap_or_else(|| "unknown".to_string());
                
                let sdp_offer = if !request.body().is_empty() {
                    String::from_utf8(request.body().to_vec()).ok()
                } else {
                    None
                };
                
                // Generate session ID
                let session_id = format!("session-{}", uuid::Uuid::new_v4());
                
                // Store mapping
                self.dialog_manager.store_dialog_mapping(&session_id, dialog_id, transaction_id, request, source);
                
                Some(RvoipCrossCrateEvent::DialogToSession(
                    DialogToSessionEvent::IncomingCall {
                        session_id,
                        call_id,
                        from,
                        to,
                        sdp_offer,
                        headers: std::collections::HashMap::new(),
                    }
                ))
            }
            
            SessionCoordinationEvent::CallAnswered { dialog_id, session_answer } => {
                if let Some(session_id) = self.dialog_manager.get_session_id(&dialog_id) {
                    Some(RvoipCrossCrateEvent::DialogToSession(
                        DialogToSessionEvent::CallEstablished {
                            session_id,
                            sdp_answer: Some(session_answer),
                        }
                    ))
                } else {
                    None
                }
            }
            
            SessionCoordinationEvent::CallTerminating { dialog_id, reason } => {
                if let Some(session_id) = self.dialog_manager.get_session_id(&dialog_id) {
                    Some(RvoipCrossCrateEvent::DialogToSession(
                        DialogToSessionEvent::CallTerminated {
                            session_id,
                            reason: TerminationReason::RemoteHangup,
                        }
                    ))
                } else {
                    None
                }
            }
            
            // DTMF events would be handled separately if implemented
            // SessionCoordinationEvent doesn't have DtmfReceived yet
            
            _ => None, // Other events not yet mapped
        }
    }
}

#[async_trait]
impl CrossCrateEventHandler for DialogEventHub {
    async fn handle(&self, event: Arc<dyn CrossCrateEvent>) -> Result<()> {
        debug!("Handling cross-crate event: {}", event.event_type());
        
        // Try to downcast to RvoipCrossCrateEvent
        // Since we can't directly downcast Arc<dyn CrossCrateEvent>, we'll use the event_type
        match event.event_type() {
            "session_to_dialog" => {
                info!("Processing session-to-dialog event");
                // Handle events from session-core
                // This is where we would process InitiateCall, TerminateSession, etc.
                // For now, log that we received it
                debug!("Received session-to-dialog event");
            }
            
            "transport_to_dialog" => {
                info!("Processing transport-to-dialog event");
                // Handle events from transport layer
                debug!("Received transport-to-dialog event");
            }
            
            _ => {
                debug!("Unhandled event type: {}", event.event_type());
            }
        }
        
        Ok(())
    }
}
