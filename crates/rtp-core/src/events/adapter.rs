//! RTP Event Adapter for Global Event Coordination
//!
//! This module provides an adapter that integrates rtp-core with the global
//! event coordinator from infra-common, enabling cross-crate event communication
//! while maintaining backward compatibility with existing RTP event handling.

use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use anyhow::Result;
use async_trait::async_trait;
use tracing::{debug, info, warn, error};

use rvoip_infra_common::events::coordinator::{GlobalEventCoordinator, CrossCrateEventHandler};
use rvoip_infra_common::events::cross_crate::{
    RvoipCrossCrateEvent, RtpToMediaEvent, RtpStatistics,
    MediaToRtpEvent, CrossCrateEvent
};
use rvoip_infra_common::planes::LayerTaskManager;

use crate::api::common::events::MediaTransportEvent;

/// RTP Event Adapter that bridges local RTP events with global cross-crate events
pub struct RtpEventAdapter {
    /// Global event coordinator for cross-crate communication
    global_coordinator: Arc<GlobalEventCoordinator>,
    
    /// Task manager for event processing tasks
    task_manager: Arc<LayerTaskManager>,
    
    /// Channel for backward compatibility with existing RTP event consumers
    transport_event_sender: Arc<RwLock<Option<mpsc::Sender<MediaTransportEvent>>>>,
    
    /// Running state
    is_running: Arc<RwLock<bool>>,
}

impl RtpEventAdapter {
    /// Create a new RTP event adapter
    pub async fn new(global_coordinator: Arc<GlobalEventCoordinator>) -> Result<Self> {
        let task_manager = Arc::new(LayerTaskManager::new("rtp-events"));
        
        Ok(Self {
            global_coordinator,
            task_manager,
            transport_event_sender: Arc::new(RwLock::new(None)),
            is_running: Arc::new(RwLock::new(false)),
        })
    }
    
    /// Start the RTP event adapter
    pub async fn start(&self) -> Result<()> {
        info!("Starting RTP Event Adapter");
        
        // Subscribe to cross-crate events targeted at rtp-core
        self.setup_cross_crate_subscriptions().await?;
        
        // Start event processing tasks
        self.start_event_processing_tasks().await?;
        
        *self.is_running.write().await = true;
        info!("RTP Event Adapter started successfully");
        
        Ok(())
    }
    
    /// Stop the RTP event adapter
    pub async fn stop(&self) -> Result<()> {
        info!("Stopping RTP Event Adapter");
        
        *self.is_running.write().await = false;
        
        // Stop event processing tasks
        self.task_manager.shutdown_all().await?;
        
        info!("RTP Event Adapter stopped");
        Ok(())
    }
    
    /// Setup subscriptions to cross-crate events
    async fn setup_cross_crate_subscriptions(&self) -> Result<()> {
        debug!("Setting up cross-crate event subscriptions for rtp-core");
        
        // Subscribe to events targeted at rtp-core
        let media_to_rtp_receiver = self.global_coordinator
            .subscribe("media_to_rtp")
            .await?;
        
        debug!("Cross-crate event subscriptions setup complete for rtp-core");
        Ok(())
    }
    
    /// Start background tasks for event processing
    async fn start_event_processing_tasks(&self) -> Result<()> {
        debug!("Starting RTP event processing tasks");
        
        // Task: Process incoming cross-crate events from media-core
        let coordinator = self.global_coordinator.clone();
        
        self.task_manager.spawn_tracked(
            "rtp-cross-crate-handler",
            rvoip_infra_common::planes::TaskPriority::High,
            async move {
                loop {
                    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                    debug!("Processing cross-crate events for rtp-core...");
                }
            }
        ).await?;
        
        debug!("RTP event processing tasks started");
        Ok(())
    }
    
    // =============================================================================
    // BACKWARD COMPATIBILITY API - For existing RTP event handling
    // =============================================================================
    
    /// Set transport event sender for backward compatibility
    pub async fn set_transport_event_sender(&self, sender: mpsc::Sender<MediaTransportEvent>) {
        *self.transport_event_sender.write().await = Some(sender);
    }
    
    /// Publish a media transport event (backward compatibility + cross-crate)
    pub async fn publish_transport_event(&self, event: MediaTransportEvent) -> Result<()> {
        // Convert to cross-crate event if applicable
        if let Some(cross_crate_event) = self.convert_transport_to_cross_crate_event(&event) {
            // Publish cross-crate event
            if let Err(e) = self.global_coordinator.publish(Arc::new(cross_crate_event)).await {
                error!("Failed to publish cross-crate event from rtp-core: {}", e);
            }
        }
        
        // Publish locally for backward compatibility
        if let Some(sender) = &*self.transport_event_sender.read().await {
            if let Err(e) = sender.try_send(event) {
                warn!("Failed to send transport event locally: {}", e);
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
    
    /// Convert local RTP transport events to cross-crate events where applicable
    fn convert_transport_to_cross_crate_event(&self, event: &MediaTransportEvent) -> Option<RvoipCrossCrateEvent> {
        match event {
            MediaTransportEvent::Connected => {
                Some(RvoipCrossCrateEvent::RtpToMedia(
                    RtpToMediaEvent::RtpStreamStarted {
                        session_id: "unknown_session".to_string(), // TODO: Get actual session ID
                        local_port: 5004,
                    }
                ))
            }
            
            MediaTransportEvent::Disconnected => {
                Some(RvoipCrossCrateEvent::RtpToMedia(
                    RtpToMediaEvent::RtpStreamStopped {
                        session_id: "unknown_session".to_string(),
                        reason: "Transport disconnected".to_string(),
                    }
                ))
            }
            
            MediaTransportEvent::QualityChanged { quality } => {
                // Convert quality metrics to RTP statistics
                Some(RvoipCrossCrateEvent::RtpToMedia(
                    RtpToMediaEvent::RtpStatisticsUpdate {
                        session_id: "unknown_session".to_string(),
                        stats: RtpStatistics {
                            packets_sent: 0,     // TODO: Get actual stats
                            packets_received: 0, // TODO: Get actual stats
                            bytes_sent: 0,       // TODO: Get actual stats
                            bytes_received: 0,   // TODO: Get actual stats
                            packet_loss_rate: 0.0, // TODO: Derive from quality
                            jitter_ms: 0.0,     // TODO: Get actual jitter
                        },
                    }
                ))
            }
            
            MediaTransportEvent::StreamEnded { ssrc, reason } => {
                Some(RvoipCrossCrateEvent::RtpToMedia(
                    RtpToMediaEvent::RtpStreamStopped {
                        session_id: format!("ssrc_{}", ssrc),
                        reason: reason.clone(),
                    }
                ))
            }
            
            MediaTransportEvent::NewStream { ssrc } => {
                Some(RvoipCrossCrateEvent::RtpToMedia(
                    RtpToMediaEvent::RtpStreamStarted {
                        session_id: format!("ssrc_{}", ssrc),
                        local_port: 5004,
                    }
                ))
            }
            
            MediaTransportEvent::Error(_) => {
                Some(RvoipCrossCrateEvent::RtpToMedia(
                    RtpToMediaEvent::RtpError {
                        session_id: "unknown_session".to_string(),
                        error: "RTP transport error".to_string(),
                    }
                ))
            }
            
            _ => None, // Not all transport events need to be cross-crate events
        }
    }
    
    /// Convert cross-crate events to local RTP transport events
    fn convert_cross_crate_to_transport_event(&self, event: &RvoipCrossCrateEvent) -> Option<MediaTransportEvent> {
        match event {
            RvoipCrossCrateEvent::MediaToRtp(media_event) => {
                match media_event {
                    MediaToRtpEvent::StartRtpStream { session_id, local_port, remote_address, remote_port, .. } => {
                        // Starting RTP stream maps to Connected event
                        Some(MediaTransportEvent::Connected)
                    }
                    
                    MediaToRtpEvent::StopRtpStream { session_id } => {
                        Some(MediaTransportEvent::Disconnected)
                    }
                    
                    MediaToRtpEvent::SendRtpPacket { session_id, payload, .. } => {
                        // No direct equivalent in MediaTransportEvent for packet sending
                        None
                    }
                    
                    MediaToRtpEvent::UpdateRtpStream { session_id, .. } => {
                        // Stream update could map to a state change
                        Some(MediaTransportEvent::StateChanged("Stream updated".to_string()))
                    }
                }
            }
            
            _ => None,
        }
    }
}

/// Event handler for processing cross-crate events in rtp-core
pub struct RtpCrossCrateEventHandler {
    adapter: Arc<RtpEventAdapter>,
}

impl RtpCrossCrateEventHandler {
    pub fn new(adapter: Arc<RtpEventAdapter>) -> Self {
        Self { adapter }
    }
}

#[async_trait]
impl CrossCrateEventHandler for RtpCrossCrateEventHandler {
    async fn handle(&self, event: Arc<dyn CrossCrateEvent>) -> Result<()> {
        debug!("Handling cross-crate event in rtp-core: {}", event.event_type());
        
        // TODO: Convert cross-crate event to local RTP action and execute
        // This is where actual cross-crate to RTP integration happens
        
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rvoip_infra_common::events::coordinator::GlobalEventCoordinator;
    
    #[tokio::test]
    async fn test_rtp_adapter_creation() {
        let coordinator = Arc::new(
            GlobalEventCoordinator::monolithic()
                .await
                .expect("Failed to create coordinator")
        );
        
        let adapter = RtpEventAdapter::new(coordinator)
            .await
            .expect("Failed to create adapter");
        
        assert!(!adapter.is_running().await);
    }
    
    #[tokio::test]
    async fn test_rtp_adapter_start_stop() {
        let coordinator = Arc::new(
            GlobalEventCoordinator::monolithic()
                .await
                .expect("Failed to create coordinator")
        );
        
        let adapter = RtpEventAdapter::new(coordinator)
            .await
            .expect("Failed to create adapter");
        
        adapter.start().await.expect("Failed to start adapter");
        assert!(adapter.is_running().await);
        
        adapter.stop().await.expect("Failed to stop adapter");
        assert!(!adapter.is_running().await);
    }
}