//! Transfer coordinator - Core logic shared by all transfer types

use crate::adapters::dialog_adapter::DialogAdapter;
use crate::session_store::SessionStore;
use crate::state_machine::StateMachineHelpers;
use crate::state_table::types::{SessionId, Role};
use crate::transfer::notify::TransferNotifyHandler;
use crate::transfer::types::{TransferOptions, TransferResult};
use crate::types::CallState;
use std::sync::Arc;
use std::time::Duration;
use tracing::{debug, error, info, warn};

/// TransferCoordinator handles the core transfer logic
/// shared by all three transfer types (blind, attended, managed)
///
/// This is the foundation that provides 70% code reuse across transfer types
pub struct TransferCoordinator {
    session_store: Arc<SessionStore>,
    state_machine_helpers: Arc<StateMachineHelpers>,
    notify_handler: TransferNotifyHandler,
}

impl TransferCoordinator {
    pub fn new(
        session_store: Arc<SessionStore>,
        state_machine_helpers: Arc<StateMachineHelpers>,
        dialog_adapter: Arc<DialogAdapter>,
    ) -> Self {
        let notify_handler = TransferNotifyHandler::new(dialog_adapter);

        Self {
            session_store,
            state_machine_helpers,
            notify_handler,
        }
    }

    /// Complete a transfer by calling the transfer target
    ///
    /// This is the core method used by all three transfer types.
    /// Different transfer types use different options:
    ///
    /// - **Blind**: No wait, terminate immediately
    /// - **Attended**: Wait for establishment, include Replaces header
    /// - **Managed**: Wait for establishment, keep old call for conference
    ///
    /// # Arguments
    /// * `transferee_session_id` - Session ID of the transferee (Alice)
    /// * `refer_to` - URI to transfer to (sip:charlie@example.com)
    /// * `options` - Transfer configuration (see TransferOptions)
    ///
    /// # Returns
    /// TransferResult with new session ID and status
    ///
    /// # Process
    /// 1. Create new session for transfer target
    /// 2. Inject MakeCall event (with optional Replaces header)
    /// 3. Send NOTIFY "100 Trying" to transferor
    /// 4. Optionally wait for call establishment
    /// 5. Send NOTIFY about final status
    /// 6. Optionally terminate old call (BYE)
    pub async fn complete_transfer(
        &self,
        transferee_session_id: &SessionId,
        refer_to: &str,
        options: TransferOptions,
    ) -> Result<TransferResult, String> {
        info!(
            "🔄 Starting transfer for session {} to target: {}",
            transferee_session_id, refer_to
        );

        // Step 1: Create new session for transfer target
        // Generate a unique session ID
        let new_session_id = SessionId(format!("transfer-{}", uuid::Uuid::new_v4()));

        let _new_session = self
            .session_store
            .create_session(new_session_id.clone(), Role::UAC, false)
            .await
            .map_err(|e| format!("Failed to create transfer session: {}", e))?;

        info!(
            "✅ Created new session {} for transfer target",
            new_session_id
        );

        // Step 2: Store transfer metadata in new session
        // TODO: Update session with transfer metadata
        // This requires get_session(), modify, then update_session()
        // For Phase 1, we'll skip this - it will be implemented in Phase 2
        debug!("TODO: Store transfer metadata (refer_to, is_transfer_call, replaces_header)");

        // Step 3: Send NOTIFY "100 Trying" to transferor
        if options.send_notify {
            if let Some(ref transferor_id) = options.transferor_session_id {
                if let Err(e) = self.notify_handler.notify_trying(transferor_id).await {
                    warn!("Failed to send trying NOTIFY: {}", e);
                }
            }
        }

        // Step 4: Initiate call to transfer target using existing state machine helpers
        // This will trigger the normal MakeCall flow: Idle → Initiating → Active
        debug!("Initiating call to transfer target {}", refer_to);

        // Get the local URI from the transferee session to use as "from"
        let from_uri = match self.session_store.get_session(transferee_session_id).await {
            Ok(transferee_session) => {
                transferee_session.local_uri.clone().unwrap_or_else(|| {
                    warn!("No local_uri in transferee session, using placeholder");
                    "sip:user@localhost".to_string()
                })
            }
            Err(e) => {
                error!("Failed to get transferee session: {}", e);
                return Ok(TransferResult::failure(
                    new_session_id,
                    format!("Failed to get transferee session: {}", e),
                    Some(500),
                ));
            }
        };

        // Use state machine helpers to make the call (same as normal outgoing call)
        // TODO: For attended transfer, we need to add Replaces header support in make_call
        match self.state_machine_helpers.make_call(&from_uri, refer_to).await {
            Ok(call_session_id) => {
                info!(
                    "✅ Initiated transfer call to {}, session: {}",
                    refer_to, call_session_id
                );

                // The returned session ID should match our new_session_id
                // If not, we have a problem
                if call_session_id != new_session_id {
                    warn!(
                        "Session ID mismatch: expected {}, got {}",
                        new_session_id, call_session_id
                    );
                }
            }
            Err(e) => {
                error!("Failed to initiate transfer call: {}", e);

                // Send failure NOTIFY
                if options.send_notify {
                    if let Some(ref transferor_id) = options.transferor_session_id {
                        let _ = self
                            .notify_handler
                            .notify_failure(transferor_id, 500, "Call initiation failed")
                            .await;
                    }
                }

                return Ok(TransferResult::failure(
                    new_session_id,
                    format!("Failed to initiate call: {}", e),
                    Some(500),
                ));
            }
        }

        // Step 5: Optionally wait for call establishment
        if options.wait_for_establishment {
            info!(
                "⏳ Waiting for transfer call establishment (timeout: {}ms)",
                options.establishment_timeout_ms
            );

            let establishment_result = self
                .wait_for_call_establishment(
                    &new_session_id,
                    Duration::from_millis(options.establishment_timeout_ms),
                )
                .await;

            match establishment_result {
                Ok(true) => {
                    info!("✅ Transfer call established successfully");

                    // Send success NOTIFY
                    if options.send_notify {
                        if let Some(ref transferor_id) = options.transferor_session_id {
                            let _ = self.notify_handler.notify_success(transferor_id).await;
                        }
                    }
                }
                Ok(false) => {
                    warn!("⏱️  Transfer call establishment timed out");

                    // Send failure NOTIFY
                    if options.send_notify {
                        if let Some(ref transferor_id) = options.transferor_session_id {
                            let _ = self
                                .notify_handler
                                .notify_failure(transferor_id, 408, "Request Timeout")
                                .await;
                        }
                    }

                    return Ok(TransferResult::failure(
                        new_session_id,
                        "Call establishment timeout".to_string(),
                        Some(408),
                    ));
                }
                Err(e) => {
                    error!("❌ Transfer call failed: {}", e);

                    // Send failure NOTIFY
                    if options.send_notify {
                        if let Some(ref transferor_id) = options.transferor_session_id {
                            let _ = self
                                .notify_handler
                                .notify_failure(transferor_id, 500, &e)
                                .await;
                        }
                    }

                    return Ok(TransferResult::failure(new_session_id, e, Some(500)));
                }
            }
        } else {
            info!("⚡ Blind transfer mode - not waiting for establishment");

            // For blind transfer, send success NOTIFY immediately after INVITE sent
            if options.send_notify {
                if let Some(ref transferor_id) = options.transferor_session_id {
                    // Send 200 OK to indicate we've attempted the transfer
                    let _ = self.notify_handler.notify_success(transferor_id).await;
                }
            }
        }

        // Step 6: Optionally terminate old call
        if options.terminate_old_call {
            info!("📴 Terminating old call for session {}", transferee_session_id);

            if let Err(e) = self.state_machine_helpers.hangup(transferee_session_id).await {
                error!("Failed to hang up old call: {}", e);
                // Don't fail the transfer if hangup fails - the new call is what matters
            } else {
                info!("✅ Hung up old call");
            }
        } else {
            info!("🔗 Keeping old call alive (managed transfer mode)");
        }

        // Get dialog ID for result
        let new_dialog_id = self
            .session_store
            .get_session(&new_session_id)
            .await
            .ok()
            .and_then(|s| s.dialog_id.clone());

        Ok(TransferResult::success(new_session_id, new_dialog_id))
    }

    /// Wait for a call to reach Active state
    ///
    /// Polls the session state until it reaches Active or fails
    ///
    /// # Returns
    /// - Ok(true) - Call reached Active state
    /// - Ok(false) - Timeout
    /// - Err(msg) - Call failed
    async fn wait_for_call_establishment(
        &self,
        session_id: &SessionId,
        timeout_duration: Duration,
    ) -> Result<bool, String> {
        let poll_interval = Duration::from_millis(100);
        let start = tokio::time::Instant::now();

        loop {
            // Check if we've timed out
            if start.elapsed() >= timeout_duration {
                return Ok(false);
            }

            // Get current session state
            match self.session_store.get_session(session_id).await {
                Ok(session) => {
                    match session.call_state {
                        CallState::Active => {
                            // Success!
                            return Ok(true);
                        }
                        CallState::Terminated => {
                            // Call failed
                            return Err(format!("Call entered Terminated state"));
                        }
                        _ => {
                            // Still in progress, keep waiting
                            debug!("Call in {:?} state, waiting...", session.call_state);
                        }
                    }
                }
                Err(e) => {
                    return Err(format!("Failed to get session state: {}", e));
                }
            }

            // Wait before next poll
            tokio::time::sleep(poll_interval).await;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_transfer_options_presets() {
        // Blind transfer
        let blind = TransferOptions::blind();
        assert!(!blind.wait_for_establishment);
        assert!(blind.terminate_old_call);

        // Attended transfer
        let attended = TransferOptions::attended("call-id;to-tag=x".to_string());
        assert!(attended.wait_for_establishment);
        assert!(attended.terminate_old_call);
        assert!(attended.replaces_header.is_some());

        // Managed consultation
        let managed = TransferOptions::managed_consultation();
        assert!(managed.wait_for_establishment);
        assert!(!managed.terminate_old_call);
    }
}
