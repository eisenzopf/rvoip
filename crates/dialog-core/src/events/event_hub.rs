//! Dialog Event Hub for Global Event Coordination
//!
//! This module provides the central event hub that integrates dialog-core with the global
//! event coordinator from infra-common, replacing channel-based communication.

use std::sync::Arc;
use anyhow::Result;
use async_trait::async_trait;
use tracing::{debug, info, warn, error};

use rvoip_infra_common::events::coordinator::{GlobalEventCoordinator, CrossCrateEventHandler};
use rvoip_infra_common::events::cross_crate::{
    CrossCrateEvent, RvoipCrossCrateEvent, DialogToSessionEvent, SessionToDialogEvent,
    DialogToTransportEvent, TransportToDialogEvent, CallState, TerminationReason
};
use rvoip_sip_core::{StatusCode, Request};
use crate::transaction::TransactionKey;

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
        if let Some(cross_crate_event) = self.convert_coordination_to_cross_crate(event.clone()) {
            info!("🚀 [event_hub] About to publish cross-crate event for event: {:?}", event);
            self.global_coordinator.publish(Arc::new(cross_crate_event)).await?;
            info!("✅ [event_hub] Published cross-crate event successfully");
        } else {
            info!("⚠️ [event_hub] convert_coordination_to_cross_crate returned None for: {:?}", event);
        }

        Ok(())
    }
    
    /// Publish a cross-crate event directly
    pub async fn publish_cross_crate_event(&self, event: RvoipCrossCrateEvent) -> Result<()> {
        debug!("Publishing cross-crate event directly: {:?}", event);
        self.global_coordinator.publish(Arc::new(event)).await?;
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
                self.dialog_manager.store_dialog_mapping(&session_id, dialog_id.clone(), transaction_id.clone(), request.clone(), source);
                
                // Include dialog_id in headers since IncomingCall doesn't have a dialog_id field
                let mut headers = std::collections::HashMap::new();
                headers.insert("X-Dialog-Id".to_string(), dialog_id.to_string());
                
                Some(RvoipCrossCrateEvent::DialogToSession(
                    DialogToSessionEvent::IncomingCall {
                        session_id,
                        call_id,
                        from,
                        to,
                        sdp_offer,
                        headers,
                        transaction_id: transaction_id.to_string(),
                        source_addr: source.to_string(),
                    }
                ))
            }
            
            SessionCoordinationEvent::CallAnswered { dialog_id, session_answer } => {
                info!("🔍 [event_hub] Processing CallAnswered for dialog: {}", dialog_id);
                match self.dialog_manager.get_session_id(&dialog_id) {
                    Some(session_id) => {
                        info!("✅ [event_hub] Found session ID {} for dialog {}", session_id, dialog_id);
                        Some(RvoipCrossCrateEvent::DialogToSession(
                            DialogToSessionEvent::CallEstablished {
                                session_id,
                                sdp_answer: Some(session_answer),
                            }
                        ))
                    }
                    None => {
                        warn!("❌ [event_hub] No session ID found for dialog {} in CallAnswered event", dialog_id);
                        None
                    }
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
            
            SessionCoordinationEvent::ResponseReceived { dialog_id, response, .. } => {
                // Try to get session ID from stored mapping first
                if let Some(session_id) = self.dialog_manager.get_session_id(&dialog_id) {
                    // Handle specific response codes
                    match response.status_code() {
                        200 => {
                            // 200 OK - call established
                            let sdp_answer = if !response.body().is_empty() {
                                String::from_utf8(response.body().to_vec()).ok()
                            } else {
                                None
                            };
                            
                            Some(RvoipCrossCrateEvent::DialogToSession(
                                DialogToSessionEvent::CallEstablished {
                                    session_id,
                                    sdp_answer,
                                }
                            ))
                        }
                        180 => {
                            // 180 Ringing
                            Some(RvoipCrossCrateEvent::DialogToSession(
                                DialogToSessionEvent::CallStateChanged {
                                    session_id,
                                    new_state: CallState::Ringing,
                                    reason: None,
                                }
                            ))
                        }
                        _ => None, // Other response codes not mapped for now
                    }
                } else {
                    warn!("No session ID found for dialog {:?}", dialog_id);
                    None
                }
            }
            
            SessionCoordinationEvent::TransferRequest { dialog_id, refer_to, replaces, .. } => {
                if let Some(session_id) = self.dialog_manager.get_session_id(&dialog_id) {
                    // Convert ReferTo to string
                    let refer_to_uri = refer_to.uri().to_string();

                    // Determine transfer type based on Replaces header
                    let transfer_type = if replaces.is_some() {
                        rvoip_infra_common::events::cross_crate::TransferType::Attended
                    } else {
                        rvoip_infra_common::events::cross_crate::TransferType::Blind
                    };

                    Some(RvoipCrossCrateEvent::DialogToSession(
                        DialogToSessionEvent::TransferRequested {
                            session_id,
                            refer_to: refer_to_uri,
                            transfer_type,
                        }
                    ))
                } else {
                    warn!("No session ID found for dialog {:?} in TransferRequest", dialog_id);
                    None
                }
            }

            // ACK events for state machine transitions
            SessionCoordinationEvent::AckSent { dialog_id, .. } => {
                // AckSent is primarily for UAC - session layer may need to know ACK was sent
                // but typically this isn't needed for state transitions
                // We'll pass it through in case session-core-v2 wants to track it
                debug!("AckSent event for dialog {}, converting to cross-crate format", dialog_id);
                None // For now, UAC doesn't need this event
            }

            SessionCoordinationEvent::AckReceived { dialog_id, negotiated_sdp, .. } => {
                // AckReceived is critical for UAS - dialog-core received ACK, now session must transition
                if let Some(session_id) = self.dialog_manager.get_session_id(&dialog_id) {
                    info!("Converting AckReceived to cross-crate event for session {}", session_id);
                    Some(RvoipCrossCrateEvent::DialogToSession(
                        DialogToSessionEvent::AckReceived {
                            session_id,
                            sdp: negotiated_sdp,
                        }
                    ))
                } else {
                    warn!("No session ID found for dialog {:?} in AckReceived", dialog_id);
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
        
        // Since we can't directly downcast Arc<dyn CrossCrateEvent>, we'll use the
        // event_type() to determine what kind of event it is and parse accordingly.
        // This is a workaround until we have proper downcast support.
        
        // Try to extract the event data from the debug representation
        let event_str = format!("{:?}", event);
        
        match event.event_type() {
            "session_to_dialog" => {
                info!("Processing session-to-dialog event: {}", event_str);
                
                // Handle StoreDialogMapping event
                if event_str.contains("StoreDialogMapping") {
                    // Extract session_id and dialog_id
                    if let Some(session_id_start) = event_str.find("session_id: \"") {
                        let session_id_content_start = session_id_start + 13;
                        if let Some(session_id_end) = event_str[session_id_content_start..].find("\"") {
                            let session_id = event_str[session_id_content_start..session_id_content_start + session_id_end].to_string();
                            
                            if let Some(dialog_id_start) = event_str.find("dialog_id: \"") {
                                let dialog_id_content_start = dialog_id_start + 12;
                                if let Some(dialog_id_end) = event_str[dialog_id_content_start..].find("\"") {
                                    let dialog_id = event_str[dialog_id_content_start..dialog_id_content_start + dialog_id_end].to_string();
                                    
                                    info!("Storing dialog mapping: session {} -> dialog {}", session_id, dialog_id);
                                    
                                    // Parse dialog ID from UUID string
                                    if let Ok(uuid) = dialog_id.parse::<uuid::Uuid>() {
                                        let parsed_dialog_id = DialogId(uuid);
                                        // Store the bidirectional mapping
                                        self.dialog_manager.session_to_dialog.insert(session_id.clone(), parsed_dialog_id.clone());
                                        self.dialog_manager.dialog_to_session.insert(parsed_dialog_id, session_id.clone());
                                        
                                        info!("Successfully stored dialog mapping for session {}", session_id);
                                    } else {
                                        warn!("Failed to parse dialog ID: {}", dialog_id);
                                    }
                                }
                            }
                        }
                    }
                }
            }
            
            "transport_to_dialog" => {
                info!("Processing transport-to-dialog event");
                // Handle transport events if needed
            }
            
            _ => {
                debug!("Unhandled event type: {}", event.event_type());
            }
        }
        
        Ok(())
    }
}
