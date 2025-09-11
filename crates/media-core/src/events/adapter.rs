//! Media Event Adapter for Global Event Coordination
//!
//! This module provides an adapter that integrates media-core with the global
//! event coordinator from infra-common, enabling cross-crate event communication
//! while maintaining backward compatibility with existing media event handling.

use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use anyhow::Result;
use async_trait::async_trait;
use tracing::{debug, info, warn, error};

use rvoip_infra_common::events::coordinator::{GlobalEventCoordinator, CrossCrateEventHandler};
use rvoip_infra_common::events::cross_crate::{
    RvoipCrossCrateEvent, MediaToSessionEvent, MediaToRtpEvent,
    SessionToMediaEvent, RtpToMediaEvent, CrossCrateEvent,
    MediaQualityMetrics, MediaStreamConfig, RecordingFormat
};
use rvoip_infra_common::planes::LayerTaskManager;

use crate::session::events::{MediaSessionEventType, QualitySeverity};
use crate::integration::events::IntegrationEventType;
use crate::types::{MediaSessionId, DialogId};

/// Media Event Adapter that bridges local media events with global cross-crate events
pub struct MediaEventAdapter {
    /// Global event coordinator for cross-crate communication
    global_coordinator: Arc<GlobalEventCoordinator>,
    
    /// Task manager for event processing tasks
    task_manager: Arc<LayerTaskManager>,
    
    /// Running state
    is_running: Arc<RwLock<bool>>,
}

impl MediaEventAdapter {
    /// Create a new media event adapter
    pub async fn new(global_coordinator: Arc<GlobalEventCoordinator>) -> Result<Self> {
        let task_manager = Arc::new(LayerTaskManager::new("media-events"));
        
        Ok(Self {
            global_coordinator,
            task_manager,
            is_running: Arc::new(RwLock::new(false)),
        })
    }
    
    /// Start the media event adapter
    pub async fn start(&self) -> Result<()> {
        info!("Starting Media Event Adapter");
        
        // Subscribe to cross-crate events targeted at media-core
        self.setup_cross_crate_subscriptions().await?;
        
        // Start event processing tasks
        self.start_event_processing_tasks().await?;
        
        *self.is_running.write().await = true;
        info!("Media Event Adapter started successfully");
        
        Ok(())
    }
    
    /// Stop the media event adapter
    pub async fn stop(&self) -> Result<()> {
        info!("Stopping Media Event Adapter");
        
        *self.is_running.write().await = false;
        
        // Stop event processing tasks
        self.task_manager.shutdown_all().await?;
        
        info!("Media Event Adapter stopped");
        Ok(())
    }
    
    /// Setup subscriptions to cross-crate events
    async fn setup_cross_crate_subscriptions(&self) -> Result<()> {
        debug!("Setting up cross-crate event subscriptions for media-core");
        
        // Subscribe to events targeted at media-core
        let session_to_media_receiver = self.global_coordinator
            .subscribe("session_to_media")
            .await?;
            
        let rtp_to_media_receiver = self.global_coordinator
            .subscribe("rtp_to_media")
            .await?;
        
        debug!("Cross-crate event subscriptions setup complete for media-core");
        Ok(())
    }
    
    /// Start background tasks for event processing
    async fn start_event_processing_tasks(&self) -> Result<()> {
        debug!("Starting media event processing tasks");
        
        // Task: Process incoming cross-crate events from session-core and rtp-core
        let coordinator = self.global_coordinator.clone();
        
        self.task_manager.spawn_tracked(
            "media-cross-crate-handler",
            rvoip_infra_common::planes::TaskPriority::High,
            async move {
                loop {
                    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                    debug!("Processing cross-crate events for media-core...");
                }
            }
        ).await?;
        
        debug!("Media event processing tasks started");
        Ok(())
    }
    
    // =============================================================================
    // BACKWARD COMPATIBILITY API - For existing media event handling
    // =============================================================================
    
    
    /// Publish a media session event (cross-crate only)
    pub async fn publish_media_event(&self, event: MediaSessionEventType) -> Result<()> {
        // Convert to cross-crate event if applicable
        if let Some(cross_crate_event) = self.convert_media_to_cross_crate_event(&event) {
            // Publish cross-crate event
            if let Err(e) = self.global_coordinator.publish(Arc::new(cross_crate_event)).await {
                error!("Failed to publish cross-crate event from media-core: {}", e);
            }
        }
        
        Ok(())
    }
    
    /// Publish an integration event (cross-crate only)
    pub async fn publish_integration_event(&self, event: IntegrationEventType) -> Result<()> {
        // Convert to cross-crate event if applicable
        if let Some(cross_crate_event) = self.convert_integration_to_cross_crate_event(&event) {
            // Publish cross-crate event
            if let Err(e) = self.global_coordinator.publish(Arc::new(cross_crate_event)).await {
                error!("Failed to publish cross-crate integration event from media-core: {}", e);
            }
        }
        
        Ok(())
    }
    
    /// Check if adapter is running
    pub async fn is_running(&self) -> bool {
        *self.is_running.read().await
    }
    
    // =============================================================================
    // CROSS-CRATE EVENT CONVERSION
    // =============================================================================
    
    /// Convert local media events to cross-crate events where applicable
    fn convert_media_to_cross_crate_event(&self, event: &MediaSessionEventType) -> Option<RvoipCrossCrateEvent> {
        match event {
            MediaSessionEventType::SessionCreated => {
                // Convert to MediaStreamStarted event - simplified
                Some(RvoipCrossCrateEvent::MediaToSession(
                    MediaToSessionEvent::MediaStreamStarted {
                        session_id: "unknown_session".to_string(), // TODO: Get actual session ID
                        local_port: 5004,
                        codec: "PCMU".to_string(),
                    }
                ))
            }
            
            MediaSessionEventType::SessionDestroyed => {
                Some(RvoipCrossCrateEvent::MediaToSession(
                    MediaToSessionEvent::MediaStreamStopped {
                        session_id: "unknown_session".to_string(),
                        reason: "Session destroyed".to_string(),
                    }
                ))
            }
            
            MediaSessionEventType::QualityIssue { metrics, severity } => {
                let mos_score = match severity {
                    QualitySeverity::Minor => 3.5,
                    QualitySeverity::Moderate => 3.0,
                    QualitySeverity::Severe => 2.5,
                    QualitySeverity::Critical => 1.5,
                };
                
                Some(RvoipCrossCrateEvent::MediaToSession(
                    MediaToSessionEvent::MediaQualityUpdate {
                        session_id: "unknown_session".to_string(),
                        quality_metrics: MediaQualityMetrics {
                            mos_score,
                            packet_loss: 0.0, // TODO: Extract from metrics
                            jitter_ms: 0.0,   // TODO: Extract from metrics
                            delay_ms: 0,      // TODO: Extract from metrics
                        },
                    }
                ))
            }
            
            MediaSessionEventType::ProcessingError { error_type, details } => {
                Some(RvoipCrossCrateEvent::MediaToSession(
                    MediaToSessionEvent::MediaError {
                        session_id: "unknown_session".to_string(),
                        error: format!("{}: {}", error_type, details),
                        error_code: None,
                    }
                ))
            }
            
            _ => None, // Not all media events need to be cross-crate events
        }
    }
    
    /// Convert integration events to cross-crate events
    fn convert_integration_to_cross_crate_event(&self, event: &IntegrationEventType) -> Option<RvoipCrossCrateEvent> {
        match event {
            IntegrationEventType::MediaSessionReady { session_id, .. } => {
                Some(RvoipCrossCrateEvent::MediaToSession(
                    MediaToSessionEvent::MediaStreamStarted {
                        session_id: session_id.to_string(),
                        local_port: 5004,
                        codec: "PCMU".to_string(),
                    }
                ))
            }
            
            IntegrationEventType::MediaSessionDestroyed { session_id } => {
                Some(RvoipCrossCrateEvent::MediaToSession(
                    MediaToSessionEvent::MediaStreamStopped {
                        session_id: session_id.to_string(),
                        reason: "Media session destroyed".to_string(),
                    }
                ))
            }
            
            IntegrationEventType::RtpSessionRegister { session_id, .. } => {
                Some(RvoipCrossCrateEvent::MediaToRtp(
                    MediaToRtpEvent::StartRtpStream {
                        session_id: session_id.to_string(),
                        local_port: 5004,
                        remote_address: "0.0.0.0".to_string(), // TODO: Get actual remote address
                        remote_port: 5004,
                        payload_type: 0,
                        codec: "PCMU".to_string(),
                    }
                ))
            }
            
            IntegrationEventType::RtpSessionUnregister { session_id } => {
                Some(RvoipCrossCrateEvent::MediaToRtp(
                    MediaToRtpEvent::StopRtpStream {
                        session_id: session_id.to_string(),
                    }
                ))
            }
            
            _ => None,
        }
    }
    
    /// Convert cross-crate events to local media events
    fn convert_cross_crate_to_media_event(&self, event: &RvoipCrossCrateEvent) -> Option<MediaSessionEventType> {
        match event {
            RvoipCrossCrateEvent::SessionToMedia(session_event) => {
                match session_event {
                    SessionToMediaEvent::StartMediaStream { session_id, .. } => {
                        Some(MediaSessionEventType::SessionCreated)
                    }
                    
                    SessionToMediaEvent::StopMediaStream { session_id } => {
                        Some(MediaSessionEventType::SessionDestroyed)
                    }
                    
                    SessionToMediaEvent::HoldMedia { session_id } => {
                        // No direct equivalent in MediaSessionEventType
                        None
                    }
                    
                    SessionToMediaEvent::ResumeMedia { session_id } => {
                        // No direct equivalent in MediaSessionEventType
                        None
                    }
                    
                    _ => None,
                }
            }
            
            _ => None,
        }
    }
}

/// Event handler for processing cross-crate events in media-core
pub struct MediaCrossCrateEventHandler {
    adapter: Arc<MediaEventAdapter>,
}

impl MediaCrossCrateEventHandler {
    pub fn new(adapter: Arc<MediaEventAdapter>) -> Self {
        Self { adapter }
    }
}

#[async_trait]
impl CrossCrateEventHandler for MediaCrossCrateEventHandler {
    async fn handle(&self, event: Arc<dyn CrossCrateEvent>) -> Result<()> {
        debug!("Handling cross-crate event in media-core: {}", event.event_type());
        
        // TODO: Convert cross-crate event to local media action and execute
        // This is where actual cross-crate to media integration happens
        
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rvoip_infra_common::events::coordinator::GlobalEventCoordinator;
    
    #[tokio::test]
    async fn test_media_adapter_creation() {
        let coordinator = Arc::new(
            rvoip_infra_common::events::global_coordinator()
                .await
                .expect("Failed to create coordinator")
        );
        
        let adapter = MediaEventAdapter::new(coordinator)
            .await
            .expect("Failed to create adapter");
        
        assert!(!adapter.is_running().await);
    }
    
    #[tokio::test]
    async fn test_media_adapter_start_stop() {
        let coordinator = Arc::new(
            rvoip_infra_common::events::global_coordinator()
                .await
                .expect("Failed to create coordinator")
        );
        
        let adapter = MediaEventAdapter::new(coordinator)
            .await
            .expect("Failed to create adapter");
        
        adapter.start().await.expect("Failed to start adapter");
        assert!(adapter.is_running().await);
        
        adapter.stop().await.expect("Failed to stop adapter");
        assert!(!adapter.is_running().await);
    }
}