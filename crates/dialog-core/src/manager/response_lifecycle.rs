//! Response Lifecycle Management
//!
//! This module provides a unified approach to dialog state management for both
//! UAC (receiving responses) and UAS (sending responses).
//!
//! ## Design Philosophy
//!
//! Dialog state transitions should happen at consistent points in the message lifecycle:
//! - **UAC**: After receiving a response (learns remote tag)
//! - **UAS**: Before sending a response (generates local tag)
//!
//! Both cases follow the same pattern: extract/generate tags, update dialog state,
//! and register in the lookup table when transitioning to Confirmed.
//!
//! ## Architecture
//!
//! ```text
//! UAC Flow (Alice):
//!   INVITE sent → Early dialog
//!   ↓
//!   200 OK received → handle_response_received() → Confirmed + lookup registered
//!
//! UAS Flow (Bob):
//!   INVITE received → Early dialog
//!   ↓
//!   200 OK built → pre_send_response() → Confirmed + lookup registered
//!   ↓
//!   200 OK sent
//! ```

use tracing::{debug, info, warn};
use rvoip_sip_core::{Request, Response, Method};

use crate::dialog::{DialogId, DialogState};
use crate::transaction::TransactionKey;
use crate::errors::{DialogResult, DialogError};
use crate::manager::core::DialogManager;
use crate::manager::utils::DialogUtils;

/// Response lifecycle hooks for dialog state management
///
/// This trait defines lifecycle hooks that are called at critical points when
/// sending or receiving responses, allowing for consistent dialog state management.
pub trait ResponseLifecycle {
    /// Called BEFORE sending a response (UAS perspective)
    ///
    /// This hook allows the dialog to be updated based on the response that's about
    /// to be sent. This is particularly important for 200 OK responses to INVITE,
    /// where the UAS needs to:
    /// 1. Extract the local tag from the response
    /// 2. Transition the dialog from Early to Confirmed
    /// 3. Register the dialog in the lookup table
    ///
    /// # Arguments
    /// * `dialog_id` - The dialog this response belongs to
    /// * `response` - The response about to be sent
    /// * `transaction_id` - The transaction this response is for
    /// * `original_request` - The original request being responded to
    ///
    /// # Returns
    /// Ok(()) if the pre-send processing succeeded
    fn pre_send_response(
        &self,
        dialog_id: &DialogId,
        response: &Response,
        transaction_id: &TransactionKey,
        original_request: &Request,
    ) -> impl std::future::Future<Output = DialogResult<()>> + Send;

    /// Called AFTER sending a response (UAS perspective)
    ///
    /// This hook can be used for post-send actions like logging, metrics, etc.
    /// Currently a no-op but provided for symmetry and future extensibility.
    ///
    /// # Arguments
    /// * `dialog_id` - The dialog this response belongs to
    /// * `response` - The response that was sent
    fn post_send_response(
        &self,
        dialog_id: &DialogId,
        response: &Response,
    ) -> impl std::future::Future<Output = DialogResult<()>> + Send;
}

/// Implementation of response lifecycle for DialogManager
impl ResponseLifecycle for DialogManager {
    /// Pre-send hook for UAS responses
    ///
    /// Handles dialog state transitions when sending responses, particularly
    /// for 200 OK responses to INVITE which confirm the dialog.
    async fn pre_send_response(
        &self,
        dialog_id: &DialogId,
        response: &Response,
        _transaction_id: &TransactionKey,
        original_request: &Request,
    ) -> DialogResult<()> {
        debug!("pre_send_response: dialog={}, status={}, method={}",
               dialog_id, response.status_code(), original_request.method());

        // Only process 200 OK responses to INVITE (dialog-confirming)
        if response.status_code() == 200 && original_request.method() == Method::Invite {
            self.confirm_uas_dialog(dialog_id, response).await?;
        }

        Ok(())
    }

    /// Post-send hook for UAS responses
    async fn post_send_response(
        &self,
        _dialog_id: &DialogId,
        _response: &Response,
    ) -> DialogResult<()> {
        // Currently a no-op, but provided for future extensibility
        Ok(())
    }
}

/// Helper methods for dialog confirmation
impl DialogManager {
    /// Confirm a UAS dialog when sending 200 OK to INVITE
    ///
    /// This method handles the dialog state transition from Early to Confirmed
    /// for UAS (server) dialogs. It:
    /// 1. Extracts the local tag from the 200 OK response
    /// 2. Updates the dialog's local_tag field
    /// 3. Transitions the dialog state to Confirmed
    /// 4. Registers the dialog in the lookup table
    ///
    /// # Arguments
    /// * `dialog_id` - The dialog to confirm
    /// * `response` - The 200 OK response being sent
    ///
    /// # Returns
    /// Ok(()) if confirmation succeeded, Err if dialog not found or invalid state
    async fn confirm_uas_dialog(
        &self,
        dialog_id: &DialogId,
        response: &Response,
    ) -> DialogResult<()> {
        debug!("Confirming UAS dialog {} (sending 200 OK to INVITE)", dialog_id);

        // Extract the local tag from the response's To header
        let local_tag = response.to()
            .and_then(|to_header| to_header.tag())
            .map(|tag| tag.to_string());

        // Get mutable access to the dialog
        let mut dialog = self.get_dialog_mut(dialog_id)?;

        // Set local tag if not already set
        if dialog.local_tag.is_none() {
            if let Some(tag) = local_tag {
                debug!("Setting local tag {} for UAS dialog {}", tag, dialog_id);
                dialog.local_tag = Some(tag.clone());
            } else {
                warn!("200 OK response missing To tag for dialog {}", dialog_id);
                return Err(DialogError::protocol_error(
                    "200 OK to INVITE must have To tag for dialog confirmation"
                ));
            }
        }

        // Transition from Early to Confirmed
        if dialog.state == DialogState::Early {
            let old_state = dialog.state.clone();
            dialog.state = DialogState::Confirmed;
            info!("🎯 Dialog {} transitioned Early → Confirmed (UAS sending 200 OK)", dialog_id);

            // Register in dialog lookup table now that we have both tags
            if let Some(tuple) = dialog.dialog_id_tuple() {
                let key = DialogUtils::create_lookup_key(&tuple.0, &tuple.1, &tuple.2);
                debug!("Registering UAS dialog in lookup table: call-id={}, local={}, remote={}",
                       tuple.0, tuple.1, tuple.2);
                self.dialog_lookup.insert(key.clone(), dialog_id.clone());
                info!("✅ Registered UAS dialog {} in lookup table with key: {}", dialog_id, key);
            } else {
                warn!("Dialog {} missing tags after 200 OK - cannot register in lookup table", dialog_id);
                return Err(DialogError::protocol_error(
                    "Dialog missing local or remote tag after confirmation"
                ));
            }

            // Emit dialog state change event
            drop(dialog); // Release the lock before emitting event

            if let Some(ref coordinator) = self.session_coordinator.read().await.as_ref() {
                let event = crate::events::SessionCoordinationEvent::DialogStateChanged {
                    dialog_id: dialog_id.clone(),
                    new_state: "Confirmed".to_string(),
                    previous_state: format!("{:?}", old_state),
                };

                if let Err(e) = coordinator.send(event).await {
                    warn!("Failed to send dialog state change event: {}", e);
                }
            }
        } else {
            debug!("Dialog {} already in {:?} state, not transitioning", dialog_id, dialog.state);
        }

        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_response_lifecycle_trait_exists() {
        // This test just validates the trait compiles
        // Actual functionality tests would require a full DialogManager setup
    }
}
