//! Event Router - Action executor for state machine transitions (v2)
//!
//! Routes actions from the state machine to the appropriate adapters.

use std::sync::Arc;
use crate::{
    state_table::types::{SessionId, EventType, Action},
    session_store_v2::SessionStore,
    errors_v2::Result,
};
use super::{
    dialog_adapter::DialogAdapter,
    media_adapter::MediaAdapter,
};

/// Routes events and actions between adapters and state machine
#[allow(dead_code)]
pub struct EventRouter {
    // Note: StateMachine will be added when executor.rs is merged
    store: Arc<SessionStore>,
    pub dialog_adapter: Arc<DialogAdapter>,
    media_adapter: Arc<MediaAdapter>,
}

impl EventRouter {
    pub fn new(
        store: Arc<SessionStore>,
        dialog_adapter: Arc<DialogAdapter>,
        media_adapter: Arc<MediaAdapter>,
    ) -> Self {
        Self {
            store,
            dialog_adapter,
            media_adapter,
        }
    }

    pub async fn start(&self) -> Result<()> {
        self.dialog_adapter.start().await?;
        Ok(())
    }

    pub async fn execute_action(&self, session_id: &SessionId, action: &Action) -> Result<()> {
        tracing::debug!("Executing action {:?} for session {}", action, session_id);
        // Action execution routing - placeholder for now
        // Full implementation would match on all Action variants
        Ok(())
    }
}
