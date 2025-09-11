//! Session Event Adapter for Global Event Coordination
//!
//! This module provides an adapter that integrates session-core with the global
//! event coordinator from infra-common, enabling cross-crate event communication
//! while maintaining backward compatibility with existing SessionEventProcessor.

use std::sync::Arc;
use tokio::sync::{mpsc, RwLock, broadcast};
use anyhow::Result;
use tracing::{debug, info, warn, error};

use rvoip_infra_common::events::coordinator::{GlobalEventCoordinator, CrossCrateEventHandler};
use rvoip_infra_common::events::cross_crate::{
    RvoipCrossCrateEvent, SessionToDialogEvent, SessionToMediaEvent, 
    DialogToSessionEvent, MediaToSessionEvent, CrossCrateEvent
};
use rvoip_infra_common::planes::LayerTaskManager;

use crate::manager::events::{SessionEvent, SessionEventProcessor, SessionEventSubscriber};
use crate::api::types::SessionId;
use crate::errors::SessionError;

/// Session Event Adapter that bridges local session events with global cross-crate events
pub struct SessionEventAdapter {
    /// Original local event processor for backward compatibility
    local_processor: Arc<SessionEventProcessor>,
    
    /// Global event coordinator for cross-crate communication
    global_coordinator: Arc<GlobalEventCoordinator>,
    
    /// Task manager for event processing tasks
    task_manager: Arc<LayerTaskManager>,
    
    /// Channel for receiving cross-crate events targeted at session-core
    cross_crate_receiver: Arc<RwLock<Option<mpsc::Receiver<Arc<dyn CrossCrateEvent>>>>>,
    
    /// Running state
    is_running: Arc<RwLock<bool>>,
}

impl SessionEventAdapter {
    /// Create a new session event adapter
    pub async fn new(global_coordinator: Arc<GlobalEventCoordinator>) -> Result<Self> {
        let local_processor = Arc::new(SessionEventProcessor::new());
        let task_manager = Arc::new(LayerTaskManager::new("session-events"));
        
        Ok(Self {
            local_processor,
            global_coordinator,
            task_manager,
            cross_crate_receiver: Arc::new(RwLock::new(None)),
            is_running: Arc::new(RwLock::new(false)),
        })
    }
    
    /// Start the event adapter
    pub async fn start(&self) -> Result<()> {
        info!("Starting Session Event Adapter");
        
        // Start the local processor
        self.local_processor.start().await
            .map_err(|e| anyhow::anyhow!("Failed to start local event processor: {}", e))?;
        
        // Subscribe to cross-crate events targeted at session-core
        self.setup_cross_crate_subscriptions().await?;
        
        // Start event processing tasks
        self.start_event_processing_tasks().await?;
        
        *self.is_running.write().await = true;
        info!("Session Event Adapter started successfully");
        
        Ok(())
    }
    
    /// Stop the event adapter
    pub async fn stop(&self) -> Result<()> {
        info!("Stopping Session Event Adapter");
        
        *self.is_running.write().await = false;
        
        // Stop event processing tasks
        self.task_manager.shutdown_all().await?;
        
        // Stop local processor
        self.local_processor.stop().await
            .map_err(|e| anyhow::anyhow!("Failed to stop local event processor: {}", e))?;
        
        info!("Session Event Adapter stopped");
        Ok(())
    }
    
    /// Setup subscriptions to cross-crate events
    async fn setup_cross_crate_subscriptions(&self) -> Result<()> {
        debug!("Setting up cross-crate event subscriptions");
        
        // Subscribe to events targeted at session-core
        let dialog_to_session_receiver = self.global_coordinator
            .subscribe("dialog_to_session")
            .await?;
            
        let media_to_session_receiver = self.global_coordinator
            .subscribe("media_to_session") 
            .await?;
        
        // Store receivers for processing
        *self.cross_crate_receiver.write().await = Some(dialog_to_session_receiver);
        
        debug!("Cross-crate event subscriptions setup complete");
        Ok(())
    }
    
    /// Start background tasks for event processing
    async fn start_event_processing_tasks(&self) -> Result<()> {
        debug!("Starting event processing tasks");
        
        // For Phase 2.5, we don't actually need a separate background task
        // The event adapters handle cross-crate events on-demand when they're published
        // This keeps the architecture simple and avoids unnecessary background threads
        
        debug!("Event processing tasks started (using on-demand processing)");
        Ok(())
    }
    
    // =============================================================================
    // BACKWARD COMPATIBILITY API - Delegates to local processor
    // =============================================================================
    
    /// Publish a local session event (backward compatibility)
    pub async fn publish_event(&self, event: SessionEvent) -> Result<(), SessionError> {
        // Check if this event should be converted to cross-crate event
        if let Some(cross_crate_event) = self.convert_to_cross_crate_event(&event) {
            // Publish cross-crate event
            if let Err(e) = self.global_coordinator.publish(Arc::new(cross_crate_event)).await {
                error!("Failed to publish cross-crate event: {}", e);
            }
        }
        
        // Always publish locally for backward compatibility
        self.local_processor.publish_event(event).await
    }
    
    /// Subscribe to local session events (backward compatibility)
    pub async fn subscribe(&self) -> Result<SessionEventSubscriber, SessionError> {
        self.local_processor.subscribe().await
    }
    
    /// Check if adapter is running
    pub async fn is_running(&self) -> bool {
        *self.is_running.read().await
    }
    
    // =============================================================================
    // CROSS-CRATE EVENT CONVERSION
    // =============================================================================
    
    /// Convert local session events to cross-crate events where applicable
    fn convert_to_cross_crate_event(&self, event: &SessionEvent) -> Option<RvoipCrossCrateEvent> {
        match event {
            SessionEvent::StateChanged { session_id, old_state, new_state } => {
                use crate::api::types::CallState;
                use rvoip_infra_common::events::cross_crate::{CallState as CrossCrateCallState, DialogToSessionEvent};
                
                // Convert CallState
                let cross_crate_state = match new_state {
                    CallState::Ringing => CrossCrateCallState::Ringing,
                    CallState::Active => CrossCrateCallState::Active,
                    CallState::OnHold => CrossCrateCallState::OnHold,
                    CallState::Terminating => CrossCrateCallState::Terminating,
                    CallState::Terminated => CrossCrateCallState::Terminated,
                    _ => return None, // Don't convert states that don't map
                };
                
                Some(RvoipCrossCrateEvent::DialogToSession(
                    DialogToSessionEvent::CallStateChanged {
                        session_id: session_id.0.clone(),
                        new_state: cross_crate_state,
                        reason: None,
                    }
                ))
            }
            
            SessionEvent::SessionCreated { session_id, from, to, .. } => {
                Some(RvoipCrossCrateEvent::SessionToDialog(
                    SessionToDialogEvent::InitiateCall {
                        session_id: session_id.0.clone(),
                        from: from.clone(),
                        to: to.clone(),
                        sdp_offer: None,
                        headers: std::collections::HashMap::new(),
                    }
                ))
            }
            
            SessionEvent::SessionTerminating { session_id, .. } => {
                use rvoip_infra_common::events::cross_crate::TerminationReason;
                
                Some(RvoipCrossCrateEvent::DialogToSession(
                    DialogToSessionEvent::CallTerminated {
                        session_id: session_id.0.clone(),
                        reason: TerminationReason::LocalHangup,
                    }
                ))
            }
            
            _ => {
                // Not all session events need to be cross-crate events
                None
            }
        }
    }
    
    /// Convert cross-crate events to local session events
    fn convert_from_cross_crate_event(&self, event: &RvoipCrossCrateEvent) -> Option<SessionEvent> {
        match event {
            RvoipCrossCrateEvent::DialogToSession(dialog_event) => {
                match dialog_event {
                    DialogToSessionEvent::IncomingCall { session_id, from, to, .. } => {
                        use crate::api::types::{CallState, SessionId};
                        Some(SessionEvent::SessionCreated {
                            session_id: SessionId(session_id.clone()),
                            from: from.clone(),
                            to: to.clone(),
                            call_state: CallState::Ringing,
                        })
                    }
                    
                    DialogToSessionEvent::CallStateChanged { session_id, new_state, .. } => {
                        use rvoip_infra_common::events::cross_crate::CallState as CrossCrateCallState;
                        use crate::api::types::{CallState, SessionId};
                        
                        let local_state = match new_state {
                            CrossCrateCallState::Ringing => CallState::Ringing,
                            CrossCrateCallState::Active => CallState::Active,
                            CrossCrateCallState::OnHold => CallState::OnHold,
                            CrossCrateCallState::Terminating => CallState::Terminating,
                            CrossCrateCallState::Terminated => CallState::Terminated,
                            _ => return None,
                        };
                        
                        Some(SessionEvent::StateChanged {
                            session_id: SessionId(session_id.clone()),
                            old_state: CallState::Initiating, // Default fallback since we don't have the old state
                            new_state: local_state,
                        })
                    }
                    
                    _ => None,
                }
            }
            
            RvoipCrossCrateEvent::MediaToSession(_) => {
                // Convert media events to session events
                None // TODO: Implement media event conversion
            }
            
            _ => None,
        }
    }
}

/// Event handler for processing cross-crate events in session-core
pub struct SessionCrossCrateEventHandler {
    local_processor: Arc<SessionEventProcessor>,
}

impl SessionCrossCrateEventHandler {
    pub fn new(local_processor: Arc<SessionEventProcessor>) -> Self {
        Self { local_processor }
    }
}

#[async_trait::async_trait]
impl CrossCrateEventHandler for SessionCrossCrateEventHandler {
    async fn handle(&self, event: Arc<dyn CrossCrateEvent>) -> Result<()> {
        debug!("Handling cross-crate event in session-core: {}", event.event_type());
        
        // TODO: Convert cross-crate event to local session event and publish
        // This is where the actual cross-crate to local event conversion happens
        
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rvoip_infra_common::events::coordinator::GlobalEventCoordinator;
    
    #[tokio::test]
    async fn test_adapter_creation() {
        let coordinator = Arc::new(
            GlobalEventCoordinator::monolithic()
                .await
                .expect("Failed to create coordinator")
        );
        
        let adapter = SessionEventAdapter::new(coordinator)
            .await
            .expect("Failed to create adapter");
        
        assert!(!adapter.is_running().await);
    }
    
    #[tokio::test]
    async fn test_adapter_start_stop() {
        let coordinator = Arc::new(
            GlobalEventCoordinator::monolithic()
                .await
                .expect("Failed to create coordinator")
        );
        
        let adapter = SessionEventAdapter::new(coordinator)
            .await
            .expect("Failed to create adapter");
        
        adapter.start().await.expect("Failed to start adapter");
        assert!(adapter.is_running().await);
        
        adapter.stop().await.expect("Failed to stop adapter");
        assert!(!adapter.is_running().await);
    }
}