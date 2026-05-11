//! Dialog Event Adapter for Global Event Coordination
//!
//! This module provides an adapter that integrates dialog-core with the global
//! event coordinator from infra-common, enabling cross-crate event communication
//! while maintaining backward compatibility with existing dialog event handling.

use anyhow::Result;
use std::sync::Arc;
use tokio::sync::{mpsc, RwLock};
use tracing::{debug, error, info, warn};

use rvoip_infra_common::events::coordinator::{CrossCrateEventHandler, GlobalEventCoordinator};
use rvoip_infra_common::events::cross_crate::{
    CallState as CrossCrateCallState, CrossCrateEvent, DialogToSessionEvent,
    DialogToTransportEvent, RvoipCrossCrateEvent, SessionToDialogEvent, TransportToDialogEvent,
};
use rvoip_infra_common::planes::LayerTaskManager;

use crate::dialog::{DialogId, DialogState};
use crate::errors::DialogError;
use crate::events::{DialogEvent, SessionCoordinationEvent};
use crate::manager::DialogManager;
use crate::transaction::TransactionKey;

/// Dialog Event Adapter that bridges local dialog events with global cross-crate events
pub struct DialogEventAdapter {
    /// Global event coordinator for cross-crate communication
    global_coordinator: Arc<GlobalEventCoordinator>,

    /// Task manager for event processing tasks
    task_manager: Arc<LayerTaskManager>,

    /// Running state
    is_running: Arc<RwLock<bool>>,

    /// Dialog manager for sending responses
    dialog_manager: Arc<RwLock<Option<Arc<DialogManager>>>>,
}

impl DialogEventAdapter {
    /// Create a new dialog event adapter
    pub async fn new(global_coordinator: Arc<GlobalEventCoordinator>) -> Result<Self> {
        let task_manager = Arc::new(LayerTaskManager::new("dialog-events"));

        Ok(Self {
            global_coordinator,
            task_manager,
            is_running: Arc::new(RwLock::new(false)),
            dialog_manager: Arc::new(RwLock::new(None)),
        })
    }

    /// Set the dialog manager for response handling
    pub async fn set_dialog_manager(&self, manager: Arc<DialogManager>) {
        *self.dialog_manager.write().await = Some(manager);
    }

    /// Start the dialog event adapter
    pub async fn start(&self) -> Result<()> {
        info!("Starting Dialog Event Adapter");

        // Subscribe to cross-crate events targeted at dialog-core
        self.setup_cross_crate_subscriptions().await?;

        // Start event processing tasks
        self.start_event_processing_tasks().await?;

        *self.is_running.write().await = true;
        info!("Dialog Event Adapter started successfully");

        Ok(())
    }

    /// Stop the dialog event adapter
    pub async fn stop(&self) -> Result<()> {
        info!("Stopping Dialog Event Adapter");

        *self.is_running.write().await = false;

        // Stop event processing tasks
        self.task_manager.shutdown_all().await?;

        info!("Dialog Event Adapter stopped");
        Ok(())
    }

    /// Setup subscriptions to cross-crate events
    async fn setup_cross_crate_subscriptions(&self) -> Result<()> {
        debug!("Setting up cross-crate event subscriptions for dialog-core");

        // Subscribe to events targeted at dialog-core
        let session_to_dialog_receiver = self
            .global_coordinator
            .subscribe("session_to_dialog")
            .await?;

        let transport_to_dialog_receiver = self
            .global_coordinator
            .subscribe("transport_to_dialog")
            .await?;

        debug!("Cross-crate event subscriptions setup complete for dialog-core");
        Ok(())
    }

    /// Start background tasks for event processing
    async fn start_event_processing_tasks(&self) -> Result<()> {
        debug!("Starting dialog event processing tasks");

        // Task: Process incoming SendRegisterResponse events from session-core
        let coordinator = self.global_coordinator.clone();
        let dialog_manager = self.dialog_manager.clone();

        self.task_manager
            .spawn_tracked(
                "dialog-register-response-handler",
                rvoip_infra_common::planes::TaskPriority::High,
                async move {
                    info!("🔔 Starting SendRegisterResponse event listener");

                    // Subscribe to session-to-dialog events
                    let mut receiver = match coordinator.subscribe("session_to_dialog").await {
                        Ok(rx) => rx,
                        Err(e) => {
                            error!("Failed to subscribe to session_to_dialog: {}", e);
                            return;
                        }
                    };

                    loop {
                        match receiver.recv().await {
                            Some(_event_arc) => {
                                // NOTE: SendRegisterResponse handling is done in DialogEventHub
                                // This adapter is not currently used for registration
                                debug!(
                                    "Received session_to_dialog event (handled by DialogEventHub)"
                                );
                            }
                            None => {
                                debug!("SendRegisterResponse event channel closed");
                                break;
                            }
                        }
                    }

                    info!("🛑 SendRegisterResponse event listener stopped");
                },
            )
            .await?;

        debug!("Dialog event processing tasks started");
        Ok(())
    }

    // =============================================================================
    // BACKWARD COMPATIBILITY API - For existing dialog event handling
    // =============================================================================

    /// Publish a dialog event (cross-crate only)
    pub async fn publish_dialog_event(&self, event: DialogEvent) -> Result<()> {
        // Convert to cross-crate event if applicable
        if let Some(cross_crate_event) = self.convert_dialog_to_cross_crate_event(&event) {
            // Publish cross-crate event
            if let Err(e) = self
                .global_coordinator
                .publish(Arc::new(cross_crate_event))
                .await
            {
                error!(
                    "Failed to publish cross-crate event from dialog-core: {}",
                    e
                );
            }
        }

        Ok(())
    }

    /// Publish a session coordination event (cross-crate only)
    pub async fn publish_session_coordination_event(
        &self,
        event: SessionCoordinationEvent,
    ) -> Result<()> {
        // Convert to cross-crate event if applicable
        if let Some(cross_crate_event) = self.convert_coordination_to_cross_crate_event(&event) {
            // Publish cross-crate event
            if let Err(e) = self
                .global_coordinator
                .publish(Arc::new(cross_crate_event))
                .await
            {
                error!(
                    "Failed to publish cross-crate coordination event from dialog-core: {}",
                    e
                );
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

    /// Convert local dialog events to cross-crate events where applicable
    fn convert_dialog_to_cross_crate_event(
        &self,
        event: &DialogEvent,
    ) -> Option<RvoipCrossCrateEvent> {
        match event {
            DialogEvent::StateChanged {
                dialog_id,
                old_state,
                new_state,
            } => {
                // Convert dialog state changes that affect session state
                let cross_crate_state = match new_state {
                    DialogState::Early => CrossCrateCallState::Ringing,
                    DialogState::Confirmed => CrossCrateCallState::Active,
                    DialogState::Terminated => CrossCrateCallState::Terminated,
                    _ => return None, // Don't convert states that don't map to session states
                };

                Some(RvoipCrossCrateEvent::DialogToSession(
                    DialogToSessionEvent::CallStateChanged {
                        session_id: dialog_id.to_string(), // Use dialog_id as session_id
                        new_state: cross_crate_state,
                        reason: None,
                    },
                ))
            }

            DialogEvent::Terminated { dialog_id, reason } => {
                use rvoip_infra_common::events::cross_crate::TerminationReason;

                let termination_reason = if reason.contains("timeout") {
                    TerminationReason::Timeout
                } else if reason.contains("error") {
                    TerminationReason::Error(reason.clone())
                } else {
                    TerminationReason::RemoteHangup
                };

                Some(RvoipCrossCrateEvent::DialogToSession(
                    DialogToSessionEvent::CallTerminated {
                        session_id: dialog_id.to_string(),
                        reason: termination_reason,
                    },
                ))
            }

            _ => None, // Not all dialog events need to be cross-crate events
        }
    }

    /// Convert session coordination events to cross-crate events
    fn convert_coordination_to_cross_crate_event(
        &self,
        event: &SessionCoordinationEvent,
    ) -> Option<RvoipCrossCrateEvent> {
        match event {
            SessionCoordinationEvent::IncomingCall {
                dialog_id, request, ..
            } => {
                // Extract SIP headers for from/to URIs - simplified for now
                let from = "unknown@unknown".to_string(); // TODO: Extract from SIP headers properly
                let to = "unknown@unknown".to_string(); // TODO: Extract from SIP headers properly
                let sdp_offer = None; // TODO: Extract SDP from request body

                Some(RvoipCrossCrateEvent::DialogToSession(
                    DialogToSessionEvent::IncomingCall {
                        session_id: dialog_id.to_string(),
                        call_id: dialog_id.to_string(),
                        from,
                        to,
                        sdp_offer,
                        headers: std::collections::HashMap::new(),
                        transaction_id: "unknown".to_string(), // TODO: Extract from request
                        source_addr: "unknown".to_string(),    // TODO: Extract from source
                    },
                ))
            }

            SessionCoordinationEvent::CallAnswered { dialog_id, .. } => {
                Some(RvoipCrossCrateEvent::DialogToSession(
                    DialogToSessionEvent::CallEstablished {
                        session_id: dialog_id.to_string(),
                        sdp_answer: None, // SDP answer would need to be extracted from response
                    },
                ))
            }

            _ => None,
        }
    }

    /// Convert cross-crate events to local dialog events
    fn convert_cross_crate_to_dialog_event(
        &self,
        event: &RvoipCrossCrateEvent,
    ) -> Option<DialogEvent> {
        match event {
            RvoipCrossCrateEvent::SessionToDialog(session_event) => {
                match session_event {
                    SessionToDialogEvent::InitiateCall {
                        session_id,
                        from,
                        to,
                        ..
                    } => {
                        // This would trigger dialog creation in dialog-core
                        // For now, we'll create a dialog creation event
                        let dialog_id = DialogId::new(); // Create new dialog ID
                        Some(DialogEvent::Created { dialog_id })
                    }

                    SessionToDialogEvent::TerminateSession { session_id, reason } => {
                        // Convert session ID to dialog ID (simplified approach)
                        let dialog_id = DialogId::new(); // In real implementation, would lookup by session_id
                        Some(DialogEvent::Terminated {
                            dialog_id,
                            reason: reason.clone(),
                        })
                    }

                    _ => None,
                }
            }

            _ => None,
        }
    }
}

/// Event handler for processing cross-crate events in dialog-core
pub struct DialogCrossCrateEventHandler {
    adapter: Arc<DialogEventAdapter>,
}

impl DialogCrossCrateEventHandler {
    pub fn new(adapter: Arc<DialogEventAdapter>) -> Self {
        Self { adapter }
    }
}

#[async_trait::async_trait]
impl CrossCrateEventHandler for DialogCrossCrateEventHandler {
    async fn handle(&self, event: Arc<dyn CrossCrateEvent>) -> Result<()> {
        debug!(
            "Handling cross-crate event in dialog-core: {}",
            event.event_type()
        );

        // TODO: Convert cross-crate event to local dialog action and execute
        // This is where actual cross-crate to dialog integration happens

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use rvoip_infra_common::events::coordinator::GlobalEventCoordinator;

    #[tokio::test]
    async fn test_dialog_adapter_creation() {
        let coordinator = rvoip_infra_common::events::global_coordinator()
            .await
            .clone();

        let adapter = DialogEventAdapter::new(coordinator)
            .await
            .expect("Failed to create adapter");

        assert!(!adapter.is_running().await);
    }

    #[tokio::test]
    async fn test_dialog_adapter_start_stop() {
        let coordinator = rvoip_infra_common::events::global_coordinator()
            .await
            .clone();

        let adapter = DialogEventAdapter::new(coordinator)
            .await
            .expect("Failed to create adapter");

        adapter.start().await.expect("Failed to start adapter");
        assert!(adapter.is_running().await);

        adapter.stop().await.expect("Failed to stop adapter");
        assert!(!adapter.is_running().await);
    }
}
