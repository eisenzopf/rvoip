//! Transport Event Adapter for Global Event Coordination
//!
//! This module provides an adapter that integrates sip-transport with the global
//! event coordinator from infra-common, enabling cross-crate event communication
//! while maintaining backward compatibility with existing transport event handling.

use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use anyhow::Result;
use async_trait::async_trait;
use tracing::{debug, info, warn, error};

use infra_common::events::coordinator::{GlobalEventCoordinator, CrossCrateEventHandler};
use infra_common::events::cross_crate::{
    RvoipCrossCrateEvent, TransportToDialogEvent, RegistrationStatus,
    DialogToTransportEvent, CrossCrateEvent
};
use infra_common::planes::LayerTaskManager;

use crate::transport::TransportEvent;
use rvoip_sip_core::{Method, StatusCode};

/// Transport Event Adapter that bridges local transport events with global cross-crate events
pub struct TransportEventAdapter {
    /// Global event coordinator for cross-crate communication
    global_coordinator: Arc<GlobalEventCoordinator>,
    
    /// Task manager for event processing tasks
    task_manager: Arc<LayerTaskManager>,
    
    /// Channel for backward compatibility with existing transport event consumers
    transport_event_sender: Arc<RwLock<Option<mpsc::Sender<TransportEvent>>>>,
    
    /// Running state
    is_running: Arc<RwLock<bool>>,
}

impl TransportEventAdapter {
    /// Create a new transport event adapter
    pub async fn new(global_coordinator: Arc<GlobalEventCoordinator>) -> Result<Self> {
        let task_manager = Arc::new(LayerTaskManager::new("transport-events"));
        
        Ok(Self {
            global_coordinator,
            task_manager,
            transport_event_sender: Arc::new(RwLock::new(None)),
            is_running: Arc::new(RwLock::new(false)),
        })
    }
    
    /// Start the transport event adapter
    pub async fn start(&self) -> Result<()> {
        info!("Starting Transport Event Adapter");
        
        // Subscribe to cross-crate events targeted at sip-transport
        self.setup_cross_crate_subscriptions().await?;
        
        // Start event processing tasks
        self.start_event_processing_tasks().await?;
        
        *self.is_running.write().await = true;
        info!("Transport Event Adapter started successfully");
        
        Ok(())
    }
    
    /// Stop the transport event adapter
    pub async fn stop(&self) -> Result<()> {
        info!("Stopping Transport Event Adapter");
        
        *self.is_running.write().await = false;
        
        // Stop event processing tasks
        self.task_manager.shutdown_all().await?;
        
        info!("Transport Event Adapter stopped");
        Ok(())
    }
    
    /// Setup subscriptions to cross-crate events
    async fn setup_cross_crate_subscriptions(&self) -> Result<()> {
        debug!("Setting up cross-crate event subscriptions for sip-transport");
        
        // Subscribe to events targeted at sip-transport
        let dialog_to_transport_receiver = self.global_coordinator
            .subscribe("dialog_to_transport")
            .await?;
        
        debug!("Cross-crate event subscriptions setup complete for sip-transport");
        Ok(())
    }
    
    /// Start background tasks for event processing
    async fn start_event_processing_tasks(&self) -> Result<()> {
        debug!("Starting transport event processing tasks");
        
        // Task: Process incoming cross-crate events from dialog-core
        let coordinator = self.global_coordinator.clone();
        
        self.task_manager.spawn_tracked(
            "transport-cross-crate-handler",
            infra_common::planes::TaskPriority::High,
            async move {
                loop {
                    tokio::time::sleep(tokio::time::Duration::from_secs(1)).await;
                    debug!("Processing cross-crate events for sip-transport...");
                }
            }
        ).await?;
        
        debug!("Transport event processing tasks started");
        Ok(())
    }
    
    // =============================================================================
    // BACKWARD COMPATIBILITY API - For existing transport event handling
    // =============================================================================
    
    /// Set transport event sender for backward compatibility
    pub async fn set_transport_event_sender(&self, sender: mpsc::Sender<TransportEvent>) {
        *self.transport_event_sender.write().await = Some(sender);
    }
    
    /// Publish a transport event (backward compatibility + cross-crate)
    pub async fn publish_transport_event(&self, event: TransportEvent) -> Result<()> {
        // Convert to cross-crate event if applicable
        if let Some(cross_crate_event) = self.convert_transport_to_cross_crate_event(&event) {
            // Publish cross-crate event
            if let Err(e) = self.global_coordinator.publish(Arc::new(cross_crate_event)).await {
                error!("Failed to publish cross-crate event from sip-transport: {}", e);
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
    
    /// Convert local transport events to cross-crate events where applicable
    fn convert_transport_to_cross_crate_event(&self, event: &TransportEvent) -> Option<RvoipCrossCrateEvent> {
        match event {
            TransportEvent::MessageReceived { message, source, destination } => {
                // Extract basic SIP information
                let method = if message.is_request() {
                    message.method().map(|m| m.to_string()).unwrap_or_else(|| "UNKNOWN".to_string())
                } else {
                    "RESPONSE".to_string()
                };
                
                let status_code = if message.is_response() {
                    message.status().map(|sc| sc.as_u16()).unwrap_or(0)
                } else {
                    0
                };
                
                let reason_phrase = if message.is_response() {
                    message.as_response()
                        .map(|resp| resp.reason_phrase().to_string())
                        .unwrap_or_else(|| String::new())
                } else {
                    String::new()
                };
                
                // Extract headers - simplified approach
                let headers = std::collections::HashMap::new(); // TODO: Extract actual headers
                
                let body = None; // TODO: Extract message body
                
                if message.is_request() {
                    Some(RvoipCrossCrateEvent::TransportToDialog(
                        TransportToDialogEvent::SipMessageReceived {
                            source: source.to_string(),
                            method,
                            headers,
                            body,
                            transaction_id: "unknown_transaction".to_string(), // TODO: Generate proper transaction ID
                        }
                    ))
                } else {
                    Some(RvoipCrossCrateEvent::TransportToDialog(
                        TransportToDialogEvent::SipResponseReceived {
                            transaction_id: "unknown_transaction".to_string(), // TODO: Extract transaction ID
                            status_code,
                            reason_phrase,
                            headers,
                            body,
                        }
                    ))
                }
            }
            
            TransportEvent::Error { error } => {
                Some(RvoipCrossCrateEvent::TransportToDialog(
                    TransportToDialogEvent::TransportError {
                        error: error.clone(),
                        transaction_id: None,
                    }
                ))
            }
            
            TransportEvent::Closed => {
                Some(RvoipCrossCrateEvent::TransportToDialog(
                    TransportToDialogEvent::TransportError {
                        error: "Transport closed".to_string(),
                        transaction_id: None,
                    }
                ))
            }
            
            // Shutdown events - not propagated to cross-crate events
            TransportEvent::ShutdownRequested |
            TransportEvent::ShutdownReady |
            TransportEvent::ShutdownNow |
            TransportEvent::ShutdownComplete => {
                // These are internal coordination events, not cross-crate
                None
            }
        }
    }
    
    /// Convert cross-crate events to local transport events
    fn convert_cross_crate_to_transport_event(&self, event: &RvoipCrossCrateEvent) -> Option<TransportEvent> {
        match event {
            RvoipCrossCrateEvent::DialogToTransport(dialog_event) => {
                match dialog_event {
                    DialogToTransportEvent::SendSipMessage { destination, method, headers, body, .. } => {
                        // This would typically trigger actual SIP message sending
                        // For now, we don't have a direct equivalent in TransportEvent
                        None
                    }
                    
                    DialogToTransportEvent::SendSipResponse { transaction_id, status_code, reason_phrase, headers, body } => {
                        // This would typically trigger actual SIP response sending
                        // For now, we don't have a direct equivalent in TransportEvent
                        None
                    }
                    
                    DialogToTransportEvent::RegisterEndpoint { uri, expires, contact } => {
                        // This would typically trigger SIP REGISTER handling
                        // No direct equivalent in current TransportEvent
                        None
                    }
                    
                    DialogToTransportEvent::UnregisterEndpoint { uri } => {
                        // This would typically trigger SIP unregister handling
                        // No direct equivalent in current TransportEvent
                        None
                    }
                }
            }
            
            _ => None,
        }
    }
}

/// Event handler for processing cross-crate events in sip-transport
pub struct TransportCrossCrateEventHandler {
    adapter: Arc<TransportEventAdapter>,
}

impl TransportCrossCrateEventHandler {
    pub fn new(adapter: Arc<TransportEventAdapter>) -> Self {
        Self { adapter }
    }
}

#[async_trait]
impl CrossCrateEventHandler for TransportCrossCrateEventHandler {
    async fn handle(&self, event: Arc<dyn CrossCrateEvent>) -> Result<()> {
        debug!("Handling cross-crate event in sip-transport: {}", event.event_type());
        
        // TODO: Convert cross-crate event to local transport action and execute
        // This is where actual cross-crate to transport integration happens
        
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use infra_common::events::coordinator::GlobalEventCoordinator;
    
    #[tokio::test]
    async fn test_transport_adapter_creation() {
        let coordinator = Arc::new(
            GlobalEventCoordinator::monolithic()
                .await
                .expect("Failed to create coordinator")
        );
        
        let adapter = TransportEventAdapter::new(coordinator)
            .await
            .expect("Failed to create adapter");
        
        assert!(!adapter.is_running().await);
    }
    
    #[tokio::test]
    async fn test_transport_adapter_start_stop() {
        let coordinator = Arc::new(
            GlobalEventCoordinator::monolithic()
                .await
                .expect("Failed to create coordinator")
        );
        
        let adapter = TransportEventAdapter::new(coordinator)
            .await
            .expect("Failed to create adapter");
        
        adapter.start().await.expect("Failed to start adapter");
        assert!(adapter.is_running().await);
        
        adapter.stop().await.expect("Failed to stop adapter");
        assert!(!adapter.is_running().await);
    }
}