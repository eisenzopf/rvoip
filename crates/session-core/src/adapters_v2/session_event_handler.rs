//! Session Event Handler - Central hub for ALL cross-crate event handling (v2)
//!
//! This is the ONLY place where cross-crate events are handled.

use std::sync::Arc;
use tokio::sync::mpsc;
use tokio::task::JoinHandle;
use rvoip_infra_common::events::coordinator::GlobalEventCoordinator;
use crate::state_table::types::{SessionId, EventType, Role};
use crate::errors_v2::{SessionError, Result as SessionResult};
use crate::adapters_v2::{DialogAdapter, MediaAdapter};
use crate::session_registry_v2::SessionRegistry;
use crate::state_table::types::DialogId;

/// Handler for processing cross-crate events in session-core v2
#[derive(Clone)]
#[allow(dead_code)]
pub struct SessionCrossCrateEventHandler {
    // Note: StateMachine will be added when executor.rs is merged
    global_coordinator: Arc<GlobalEventCoordinator>,
    dialog_adapter: Arc<DialogAdapter>,
    media_adapter: Arc<MediaAdapter>,
    registry: Arc<SessionRegistry>,
    incoming_call_tx: Option<mpsc::Sender<crate::types_v2::IncomingCallInfo>>,
    transfer_coordinator: Option<Arc<crate::transfer_v2::TransferCoordinator>>,
    task_handles: Arc<tokio::sync::Mutex<Vec<JoinHandle<()>>>>,
}

impl SessionCrossCrateEventHandler {
    pub fn new(
        global_coordinator: Arc<GlobalEventCoordinator>,
        dialog_adapter: Arc<DialogAdapter>,
        media_adapter: Arc<MediaAdapter>,
        registry: Arc<SessionRegistry>,
    ) -> Self {
        Self {
            global_coordinator,
            dialog_adapter,
            media_adapter,
            registry,
            incoming_call_tx: None,
            transfer_coordinator: None,
            task_handles: Arc::new(tokio::sync::Mutex::new(Vec::new())),
        }
    }

    pub fn with_incoming_call_channel(
        global_coordinator: Arc<GlobalEventCoordinator>,
        dialog_adapter: Arc<DialogAdapter>,
        media_adapter: Arc<MediaAdapter>,
        registry: Arc<SessionRegistry>,
        incoming_call_tx: mpsc::Sender<crate::types_v2::IncomingCallInfo>,
    ) -> Self {
        Self {
            global_coordinator,
            dialog_adapter,
            media_adapter,
            registry,
            incoming_call_tx: Some(incoming_call_tx),
            transfer_coordinator: None,
            task_handles: Arc::new(tokio::sync::Mutex::new(Vec::new())),
        }
    }

    pub fn set_transfer_coordinator(&mut self, coordinator: Arc<crate::transfer_v2::TransferCoordinator>) {
        self.transfer_coordinator = Some(coordinator);
    }

    pub async fn shutdown(&self) {
        let mut handles = self.task_handles.lock().await;
        for handle in handles.drain(..) {
            handle.abort();
        }
    }

    pub async fn start(&self) -> SessionResult<()> {
        // Event subscription setup - placeholder
        // Full implementation would subscribe to global events
        Ok(())
    }
}
