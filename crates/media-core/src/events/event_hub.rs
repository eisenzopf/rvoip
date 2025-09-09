//! Media Event Hub for Global Event Coordination
//!
//! This module provides the central event hub that integrates media-core with the global
//! event coordinator from infra-common, replacing channel-based communication.

use std::sync::Arc;
use anyhow::Result;
use async_trait::async_trait;
use tracing::{debug, info, warn, error};

use infra_common::events::coordinator::{GlobalEventCoordinator, CrossCrateEventHandler};
use infra_common::events::cross_crate::{
    CrossCrateEvent, RvoipCrossCrateEvent, MediaToSessionEvent, SessionToMediaEvent,
    MediaToRtpEvent, RtpToMediaEvent, MediaQualityMetrics
};

use crate::relay::controller::{MediaSessionController, MediaSessionEvent};
use crate::types::{MediaSessionId, DialogId};

/// Media Event Hub that handles all cross-crate event communication
#[derive(Clone)]
pub struct MediaEventHub {
    /// Global event coordinator for cross-crate communication
    global_coordinator: Arc<GlobalEventCoordinator>,
    
    /// Reference to media controller for handling incoming events
    media_controller: Arc<MediaSessionController>,
}

impl std::fmt::Debug for MediaEventHub {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("MediaEventHub")
            .field("global_coordinator", &"Arc<GlobalEventCoordinator>")
            .field("media_controller", &"Arc<MediaSessionController>")
            .finish()
    }
}

impl MediaEventHub {
    /// Create a new media event hub
    pub async fn new(
        global_coordinator: Arc<GlobalEventCoordinator>,
        media_controller: Arc<MediaSessionController>,
    ) -> Result<Arc<Self>> {
        let hub = Arc::new(Self {
            global_coordinator: global_coordinator.clone(),
            media_controller,
        });
        
        // Clone hub for registration
        let handler = MediaEventHub {
            global_coordinator: global_coordinator.clone(),
            media_controller: hub.media_controller.clone(),
        };
        
        // Register as handler for session-to-media events
        global_coordinator
            .register_handler("session_to_media", handler.clone())
            .await?;
            
        // Register as handler for rtp-to-media events
        global_coordinator
            .register_handler("rtp_to_media", handler)
            .await?;
        
        info!("Media Event Hub initialized and registered with GlobalEventCoordinator");
        
        Ok(hub)
    }
    
    /// Publish a media event to the global bus
    pub async fn publish_media_event(&self, event: MediaSessionEvent) -> Result<()> {
        debug!("Publishing media event: {:?}", event);
        
        // Convert to cross-crate event if applicable
        if let Some(cross_crate_event) = self.convert_media_to_cross_crate(event) {
            self.global_coordinator.publish(Arc::new(cross_crate_event)).await?;
        }
        
        Ok(())
    }
    
    /// Convert MediaSessionEvent to cross-crate event
    fn convert_media_to_cross_crate(&self, event: MediaSessionEvent) -> Option<RvoipCrossCrateEvent> {
        match event {
            MediaSessionEvent::SessionCreated { dialog_id, session_id: media_session_id } => {
                // Convert DialogId to MediaSessionId
                // DialogId is used as MediaSessionId in this context
                let media_id = MediaSessionId::from_dialog(&dialog_id);
                
                // Get session ID from media controller mapping
                if let Some(session_id) = self.media_controller.get_session_id(&media_id) {
                    // Try to get media session info for codec info
                    let codec = "PCMU".to_string(); // Default codec
                    let local_port = 5004; // Default port
                    
                    Some(RvoipCrossCrateEvent::MediaToSession(
                        MediaToSessionEvent::MediaStreamStarted {
                            session_id,
                            local_port,
                            codec,
                        }
                    ))
                } else {
                    warn!("No session ID found for dialog {:?}", dialog_id);
                    None
                }
            }
            
            MediaSessionEvent::SessionDestroyed { dialog_id, session_id: _ } => {
                // DialogId is used as MediaSessionId in this context
                let media_id = MediaSessionId::from_dialog(&dialog_id);
                if let Some(session_id) = self.media_controller.get_session_id(&media_id) {
                    Some(RvoipCrossCrateEvent::MediaToSession(
                        MediaToSessionEvent::MediaStreamStopped {
                            session_id,
                            reason: "Session destroyed".to_string(),
                        }
                    ))
                } else {
                    None
                }
            }
            
            MediaSessionEvent::SessionFailed { dialog_id, error } => {
                // DialogId is used as MediaSessionId in this context
                let media_id = MediaSessionId::from_dialog(&dialog_id);
                if let Some(session_id) = self.media_controller.get_session_id(&media_id) {
                    Some(RvoipCrossCrateEvent::MediaToSession(
                        MediaToSessionEvent::MediaError {
                            session_id,
                            error,
                            error_code: Some(500), // Generic media error code
                        }
                    ))
                } else {
                    None
                }
            }
            
            _ => None, // Other events are internal only
        }
    }
    
    
}

#[async_trait]
impl CrossCrateEventHandler for MediaEventHub {
    async fn handle(&self, event: Arc<dyn CrossCrateEvent>) -> Result<()> {
        debug!("Handling cross-crate event: {}", event.event_type());
        
        // Handle events based on type
        match event.event_type() {
            "session_to_media" => {
                info!("Processing session-to-media event");
                // Handle events from session-core
                // This is where we would process StartMediaStream, StopMediaStream, etc.
                debug!("Received session-to-media event");
            }
            
            "rtp_to_media" => {
                info!("Processing rtp-to-media event");
                // Handle events from rtp-core
                debug!("Received rtp-to-media event");
            }
            
            _ => {
                debug!("Unhandled event type: {}", event.event_type());
            }
        }
        
        Ok(())
    }
}
