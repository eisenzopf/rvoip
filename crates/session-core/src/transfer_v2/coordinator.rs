//! Transfer coordinator - Core logic shared by all transfer types (v2)

use crate::adapters_v2::dialog_adapter::DialogAdapter;
use crate::session_store_v2::SessionStore;
// Note: StateMachineHelpers will be available when helpers.rs is merged into state_machine module
use crate::state_table::types::{SessionId, Role, EventType};
use crate::transfer_v2::notify::TransferNotifyHandler;
use crate::transfer_v2::types::{TransferOptions, TransferResult};
use crate::state_table::CallState;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, error, info, warn};

/// TransferCoordinator handles the core transfer logic
pub struct TransferCoordinator {
    session_store: Arc<SessionStore>,
    // state_machine_helpers: Arc<StateMachineHelpers>, // TODO: Add when StateMachineHelpers is available
    notify_handler: TransferNotifyHandler,
}

impl TransferCoordinator {
    pub fn new(
        session_store: Arc<SessionStore>,
        // state_machine_helpers: Arc<StateMachineHelpers>, // TODO: Add when available
        dialog_adapter: Arc<DialogAdapter>,
    ) -> Self {
        let notify_handler = TransferNotifyHandler::new(dialog_adapter);
        Self {
            session_store,
            // state_machine_helpers,
            notify_handler,
        }
    }

    pub async fn complete_transfer(
        &self,
        transferee_session_id: &SessionId,
        refer_to: &str,
        options: TransferOptions,
    ) -> Result<TransferResult, String> {
        info!("Starting transfer for session {} to target: {}", transferee_session_id, refer_to);

        let new_session_id = SessionId(format!("transfer-{}", uuid::Uuid::new_v4()));

        let _new_session = self.session_store
            .create_session(new_session_id.clone(), Role::UAC, false)
            .await
            .map_err(|e| format!("Failed to create transfer session: {}", e))?;

        let transferee_session = self.session_store.get_session(transferee_session_id).await
            .map_err(|e| format!("Failed to get transferee session: {}", e))?;

        let transferor_session_id = transferee_session.transferor_session_id.clone();

        let from_uri = match self.session_store.get_session(transferee_session_id).await {
            Ok(session) => session.local_uri.clone().unwrap_or_else(|| "sip:user@localhost".to_string()),
            Err(e) => return Ok(TransferResult::failure(new_session_id, format!("Failed to get transferee session: {}", e), Some(500))),
        };

        let mut new_session = self.session_store.get_session(&new_session_id).await
            .map_err(|e| format!("Failed to get new session: {}", e))?;

        new_session.is_transfer_call = true;
        new_session.transfer_target = Some(refer_to.to_string());
        new_session.transferor_session_id = transferor_session_id;
        new_session.local_uri = Some(from_uri.clone());
        new_session.remote_uri = Some(refer_to.to_string());

        if let Some(ref replaces) = options.replaces_header {
            new_session.replaces_header = Some(replaces.clone());
        }

        self.session_store.update_session(new_session).await
            .map_err(|e| format!("Failed to update transfer session metadata: {}", e))?;

        if options.send_notify {
            if let Some(ref transferor_id) = options.transferor_session_id {
                if let Err(e) = self.notify_handler.notify_trying(transferor_id).await {
                    tracing::warn!("Failed to send transfer trying notification: {e}");
                }
            }
        }

        // TODO: When StateMachineHelpers is available, call:
        // self.state_machine_helpers.state_machine.process_event(&new_session_id, EventType::MakeCall { target: refer_to.to_string() }).await
        info!("Would initiate transfer call to {} on session {} (state machine not yet wired)", refer_to, new_session_id);

        if options.wait_for_establishment {
            let result = self.wait_for_call_establishment(
                &new_session_id,
                Duration::from_millis(options.establishment_timeout_ms),
            ).await;

            match result {
                Ok(true) => {
                    if options.send_notify {
                        if let Some(ref transferor_id) = options.transferor_session_id {
                            if let Err(e) = self.notify_handler.notify_success(transferor_id).await {
                                tracing::warn!("Failed to send transfer success notification: {e}");
                            }
                        }
                    }
                }
                Ok(false) => {
                    if options.send_notify {
                        if let Some(ref transferor_id) = options.transferor_session_id {
                            if let Err(e) = self.notify_handler.notify_failure(transferor_id, 408, "Request Timeout").await {
                                tracing::warn!("Failed to send transfer timeout notification: {e}");
                            }
                        }
                    }
                    return Ok(TransferResult::failure(new_session_id, "Call establishment timeout".to_string(), Some(408)));
                }
                Err(e) => {
                    if options.send_notify {
                        if let Some(ref transferor_id) = options.transferor_session_id {
                            if let Err(e2) = self.notify_handler.notify_failure(transferor_id, 500, &e).await {
                                tracing::warn!("Failed to send transfer failure notification: {e2}");
                            }
                        }
                    }
                    return Ok(TransferResult::failure(new_session_id, e, Some(500)));
                }
            }
        } else {
            if options.send_notify {
                if let Some(ref transferor_id) = options.transferor_session_id {
                    if let Err(e) = self.notify_handler.notify_success(transferor_id).await {
                        tracing::warn!("Failed to send transfer success notification: {e}");
                    }
                }
            }
        }

        let new_dialog_id = self.session_store
            .get_session(&new_session_id)
            .await
            .ok()
            .and_then(|s| s.dialog_id.clone());

        Ok(TransferResult::success(new_session_id, new_dialog_id))
    }

    async fn wait_for_call_establishment(
        &self,
        session_id: &SessionId,
        timeout_duration: Duration,
    ) -> Result<bool, String> {
        let poll_interval = Duration::from_millis(100);
        let start = tokio::time::Instant::now();

        loop {
            if start.elapsed() >= timeout_duration {
                return Ok(false);
            }

            match self.session_store.get_session(session_id).await {
                Ok(session) => {
                    match session.call_state {
                        CallState::Active => return Ok(true),
                        CallState::Terminated => return Err("Call entered Terminated state".to_string()),
                        _ => debug!("Call in {:?} state, waiting...", session.call_state),
                    }
                }
                Err(e) => return Err(format!("Failed to get session state: {}", e)),
            }

            tokio::time::sleep(poll_interval).await;
        }
    }
}
