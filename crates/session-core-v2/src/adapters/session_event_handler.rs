//! Session Event Handler for cross-crate event processing
//!
//! This handler subscribes to events from dialog-core and media-core through
//! the GlobalEventCoordinator and routes them to the state machine.

use std::sync::Arc;
use anyhow::Result;
use infra_common::events::coordinator::CrossCrateEventHandler;
use infra_common::events::cross_crate::{
    CrossCrateEvent, DialogToSessionEvent, MediaToSessionEvent, RvoipCrossCrateEvent,
};
use crate::state_table::types::{SessionId, EventType};
use crate::state_machine::StateMachine as StateMachineExecutor;
use tracing::{debug, info, warn, error};

/// Handler for processing cross-crate events in session-core-v2
#[derive(Clone)]
pub struct SessionCrossCrateEventHandler {
    /// State machine executor
    state_machine: Arc<StateMachineExecutor>,
}

impl SessionCrossCrateEventHandler {
    pub fn new(state_machine: Arc<StateMachineExecutor>) -> Self {
        Self { state_machine }
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
    /// Extract session ID from event debug string
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
    
    /// Convert dialog event string to EventType
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
    
    /// Convert media event string to EventType
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