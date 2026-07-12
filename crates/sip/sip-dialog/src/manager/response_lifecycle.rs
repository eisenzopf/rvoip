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
//!
//! UAS final rejection flow (Bob):
//!   INVITE received → Early dialog
//!   ↓
//!   3xx-6xx built → pre_send_response() → Terminated + early lookup removed
//!   ↓
//!   final response sent
//! ```

use rvoip_sip_core::{Method, Request, Response};
use tracing::{debug, info, warn};

use crate::diagnostics::safe_log::method_class;
use crate::dialog::{DialogId, DialogState};
use crate::errors::{DialogError, DialogResult};
use crate::manager::core::DialogManager;
use crate::manager::utils::DialogUtils;
use crate::transaction::TransactionKey;

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
    /// Handles dialog state transitions when sending responses. A 2xx final
    /// response to an initial INVITE confirms the dialog. A 3xx-6xx final
    /// response terminates any early dialog created by provisional responses;
    /// this is especially important for RFC 3261 §22.2 auth retries because
    /// the authenticated INVITE is a new initial INVITE transaction, not an
    /// in-dialog re-INVITE.
    async fn pre_send_response(
        &self,
        dialog_id: &DialogId,
        response: &Response,
        _transaction_id: &TransactionKey,
        original_request: &Request,
    ) -> DialogResult<()> {
        debug!(
            "pre_send_response: dialog={}, status={}, method={}",
            dialog_id,
            response.status_code(),
            method_class(&original_request.method())
        );

        if original_request.method() == Method::Invite {
            match response.status_code() {
                200 => self.confirm_uas_dialog(dialog_id, response).await?,
                300..=699 => self.terminate_uas_early_dialog_for_final_response(dialog_id)?,
                _ => {}
            }
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
        debug!(
            "Confirming UAS dialog {} (sending 200 OK to INVITE)",
            dialog_id
        );

        // Extract the local tag from the response's To header
        let local_tag = response
            .to()
            .and_then(|to_header| to_header.tag())
            .map(|tag| tag.to_string());

        // Get mutable access to the dialog
        let mut dialog = self.get_dialog_mut(dialog_id)?;

        // Set local tag if not already set
        if dialog.local_tag.is_none() {
            if let Some(tag) = local_tag {
                debug!("Setting local tag for UAS dialog {}", dialog_id);
                dialog.local_tag = Some(tag.clone());
            } else {
                warn!("200 OK response missing To tag for dialog {}", dialog_id);
                return Err(DialogError::protocol_error(
                    "200 OK to INVITE must have To tag for dialog confirmation",
                ));
            }
        }

        // Transition from Early to Confirmed
        if dialog.state == DialogState::Early {
            let old_state = dialog.state.clone();
            dialog.state = DialogState::Confirmed;
            info!(
                "🎯 Dialog {} transitioned Early → Confirmed (UAS sending 200 OK)",
                dialog_id
            );

            // Register in dialog lookup table now that we have both tags
            if let Some(tuple) = dialog.dialog_id_tuple() {
                if let Some(remote_tag) = dialog.remote_tag.as_ref() {
                    let early_key =
                        DialogUtils::create_early_lookup_key(&dialog.call_id, remote_tag);
                    self.early_dialog_lookup.remove(&early_key);
                }
                let key = DialogUtils::create_lookup_key(&tuple.0, &tuple.1, &tuple.2);
                debug!(
                    "Registering UAS dialog in lookup table: call-id={}, local={}, remote={}",
                    tuple.0, tuple.1, tuple.2
                );
                self.dialog_lookup.insert(key.clone(), dialog_id.clone());
                info!("✅ Registered UAS dialog {} in lookup table", dialog_id);
            } else {
                warn!(
                    "Dialog {} missing tags after 200 OK - cannot register in lookup table",
                    dialog_id
                );
                return Err(DialogError::protocol_error(
                    "Dialog missing local or remote tag after confirmation",
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

                if let Err(_error) = coordinator.send(event).await {
                    warn!("Failed to send dialog state change event");
                }
            }
        } else {
            debug!(
                "Dialog {} already in {:?} state, not transitioning",
                dialog_id, dialog.state
            );
        }

        Ok(())
    }

    /// Terminate an early UAS dialog before sending a final non-2xx response
    /// to the initial INVITE.
    ///
    /// RFC 3261 §12.3 says early dialogs terminate when a non-2xx final
    /// response is sent for the initial INVITE. Removing the early lookup here
    /// closes the race where an authenticated retry after 401/407 could arrive
    /// before upper-layer session cleanup and be misclassified as a re-INVITE.
    fn terminate_uas_early_dialog_for_final_response(
        &self,
        dialog_id: &DialogId,
    ) -> DialogResult<()> {
        let mut dialog = self.get_dialog_mut(dialog_id)?;
        if dialog.state != DialogState::Early {
            debug!(
                "Dialog {} is {:?}, not terminating as rejected early dialog",
                dialog_id, dialog.state
            );
            return Ok(());
        }

        if let Some(remote_tag) = dialog.remote_tag.as_ref() {
            let early_key = DialogUtils::create_early_lookup_key(&dialog.call_id, remote_tag);
            self.early_dialog_lookup.remove(&early_key);
        }

        dialog.state = DialogState::Terminated;
        info!(
            "Dialog {} transitioned Early -> Terminated (UAS sending final non-2xx INVITE response)",
            dialog_id
        );
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::manager::DialogLookup;
    use crate::transaction::TransactionManager;
    use async_trait::async_trait;
    use rvoip_sip_core::builder::SimpleRequestBuilder;
    use rvoip_sip_core::StatusCode;
    use rvoip_sip_transport::error::Result as TransportResult;
    use rvoip_sip_transport::{Transport, TransportEvent};
    use std::net::SocketAddr;
    use std::str::FromStr;
    use std::sync::atomic::{AtomicBool, Ordering};
    use std::sync::Arc;
    use tokio::sync::mpsc;

    #[derive(Debug)]
    struct NoopTransport {
        addr: SocketAddr,
        closed: AtomicBool,
    }

    impl NoopTransport {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                addr: SocketAddr::from_str("127.0.0.1:5060").unwrap(),
                closed: AtomicBool::new(false),
            })
        }
    }

    #[async_trait]
    impl Transport for NoopTransport {
        fn local_addr(&self) -> TransportResult<SocketAddr> {
            Ok(self.addr)
        }

        async fn send_message(
            &self,
            _message: rvoip_sip_core::Message,
            _destination: SocketAddr,
        ) -> TransportResult<()> {
            Ok(())
        }

        async fn close(&self) -> TransportResult<()> {
            self.closed.store(true, Ordering::SeqCst);
            Ok(())
        }

        fn is_closed(&self) -> bool {
            self.closed.load(Ordering::SeqCst)
        }
    }

    async fn make_manager() -> DialogManager {
        let transport = NoopTransport::new();
        let (_tx, transport_rx) = mpsc::channel::<TransportEvent>(16);
        let (transaction_manager, _events_rx) =
            TransactionManager::new(transport, transport_rx, Some(16))
                .await
                .expect("build TransactionManager");
        DialogManager::new(
            Arc::new(transaction_manager),
            SocketAddr::from_str("127.0.0.1:5060").unwrap(),
        )
        .await
        .expect("build DialogManager")
    }

    fn initial_invite() -> Request {
        SimpleRequestBuilder::new(Method::Invite, "sip:bob@example.com")
            .unwrap()
            .from("Alice", "sip:alice@example.com", Some("alice-tag"))
            .to("Bob", "sip:bob@example.com", None)
            .contact("sip:alice@127.0.0.1:5061", None)
            .call_id("auth-retry-dialog-test")
            .cseq(1)
            .via("127.0.0.1:5061", "UDP", Some("z9hG4bK-auth-retry"))
            .max_forwards(70)
            .build()
    }

    #[tokio::test]
    async fn final_non_2xx_to_initial_invite_removes_early_dialog_lookup() {
        let manager = make_manager().await;
        let request = initial_invite();
        let dialog_id = manager
            .create_early_dialog_from_invite(&request)
            .await
            .expect("create early dialog");

        assert_eq!(
            manager.find_dialog_for_request(&request).await,
            Some(dialog_id.clone()),
            "initial early dialog should be discoverable before final response"
        );

        let response = Response::new(StatusCode::Unauthorized);
        let transaction_id =
            TransactionKey::new("z9hG4bK-auth-retry".to_string(), Method::Invite, true);

        manager
            .pre_send_response(&dialog_id, &response, &transaction_id, &request)
            .await
            .expect("pre-send lifecycle");

        assert_eq!(
            manager
                .get_dialog_state(&dialog_id)
                .expect("dialog should remain until upper-layer cleanup"),
            DialogState::Terminated
        );
        assert_eq!(
            manager.find_dialog_for_request(&request).await,
            None,
            "a no-To-tag authenticated retry must not resolve as a re-INVITE"
        );
    }
}
