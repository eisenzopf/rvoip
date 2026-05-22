//! BYE Request Handler for Dialog-Core
//!
//! This module handles BYE requests according to RFC 3261 Section 15.
//! BYE requests terminate established SIP dialogs and clean up associated resources.
//!
//! ## BYE Processing Steps
//!
//! 1. **Dialog Identification**: Match BYE to existing dialog using Call-ID and tags
//! 2. **Authorization Check**: Verify BYE is from dialog participant
//! 3. **State Validation**: Ensure dialog is in confirmable state for termination
//! 4. **Resource Cleanup**: Terminate dialog and clean up associated state
//! 5. **Response Generation**: Send 200 OK to acknowledge BYE receipt
//!
//! ## Error Handling
//!
//! - **481 Call/Transaction Does Not Exist**: No matching dialog found
//! - **403 Forbidden**: BYE from unauthorized party
//! - **500 Server Internal Error**: Processing failures

use tracing::{debug, info, warn};

use crate::dialog::{Dialog, DialogId, DialogState};
use crate::errors::{DialogError, DialogResult};
use crate::events::SessionCoordinationEvent;
use crate::manager::utils::DialogUtils;
use crate::manager::{DialogManager, SourceExtractor};
use crate::transaction::{TransactionKey, utils::response_builders};
use rvoip_sip_core::{HeaderName, Request, StatusCode, TypedHeader};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum ByeSequenceDisposition {
    Fresh,
    DuplicateTerminated,
}

fn classify_bye_sequence(
    dialog: &Dialog,
    request: &Request,
) -> DialogResult<ByeSequenceDisposition> {
    let new_seq = bye_cseq(request)?;

    if dialog.state == DialogState::Terminated
        && dialog.remote_cseq != 0
        && new_seq == dialog.remote_cseq
    {
        Ok(ByeSequenceDisposition::DuplicateTerminated)
    } else {
        Ok(ByeSequenceDisposition::Fresh)
    }
}

fn bye_cseq(request: &Request) -> DialogResult<u32> {
    match request.header(&HeaderName::CSeq) {
        Some(TypedHeader::CSeq(cseq)) => Ok(cseq.sequence()),
        _ => Err(DialogError::protocol_error("Request missing CSeq header")),
    }
}

fn matches_terminated_bye_retransmit(manager: &DialogManager, request: &Request) -> bool {
    let Ok(cseq) = bye_cseq(request) else {
        return false;
    };
    let Some((call_id, Some(from_tag), Some(to_tag))) = DialogUtils::extract_dialog_info(request)
    else {
        return false;
    };

    let (key1, key2) = DialogUtils::create_bidirectional_keys(&call_id, &from_tag, &to_tag);
    for key in [key1, key2] {
        if manager
            .terminated_bye_lookup
            .get(&key)
            .is_some_and(|entry| entry.value().cseq == cseq)
        {
            return true;
        }
    }
    false
}

/// BYE-specific handling operations
pub trait ByeHandler {
    /// Handle BYE requests (dialog-terminating)
    fn handle_bye_method(
        &self,
        request: Request,
    ) -> impl std::future::Future<Output = DialogResult<()>> + Send;
}

/// Implementation of BYE handling for DialogManager
impl ByeHandler for DialogManager {
    /// Handle BYE requests according to RFC 3261 Section 15
    ///
    /// Terminates the dialog and sends appropriate responses.
    async fn handle_bye_method(&self, request: Request) -> DialogResult<()> {
        debug!("Processing BYE request");

        let source = SourceExtractor::extract_from_request(&request);

        // Create server transaction
        let server_transaction = self
            .transaction_manager
            .create_server_transaction(request.clone(), source)
            .await
            .map_err(|e| DialogError::TransactionError {
                message: format!("Failed to create server transaction for BYE: {}", e),
            })?;

        let transaction_id = server_transaction.id().clone();

        // Find the dialog for this BYE
        if let Some(dialog_id) = self.find_dialog_for_request(&request).await {
            self.process_bye_in_dialog(transaction_id, request, dialog_id)
                .await
        } else {
            let status_code = if matches_terminated_bye_retransmit(self, &request) {
                debug!("BYE retransmit matched recently terminated dialog");
                StatusCode::Ok
            } else {
                StatusCode::CallOrTransactionDoesNotExist
            };

            // RFC 3261 §15.1.2: a BYE that does not match an existing dialog
            // gets a 481 Call/Transaction Does Not Exist. This happens in
            // normal operation when a peer retransmits a BYE past our dialog
            // teardown (e.g. its 200 OK was lost), so it is not an error.
            // Recently terminated BYEs keep a compact tombstone so late
            // retransmits still receive the original idempotent 200 OK.
            let response = response_builders::create_response(&request, status_code);
            self.transaction_manager
                .send_response(&transaction_id, response)
                .await
                .map_err(|e| DialogError::TransactionError {
                    message: format!("Failed to send 481 response to BYE: {}", e),
                })?;
            if status_code == StatusCode::Ok {
                self.release_bye_server_transaction(&transaction_id).await;
            }

            debug!(
                "BYE processed with {} response (no dialog found)",
                status_code
            );
            Ok(())
        }
    }
}

/// BYE-specific helper methods for DialogManager
impl DialogManager {
    /// Process BYE within a dialog
    pub async fn process_bye_in_dialog(
        &self,
        transaction_id: TransactionKey,
        request: Request,
        dialog_id: DialogId,
    ) -> DialogResult<()> {
        debug!("Processing BYE for dialog {}", dialog_id);

        // Update dialog and terminate. A retransmitted BYE can escape the
        // transaction layer after the original BYE has already terminated the
        // dialog; answer it idempotently without re-emitting cleanup.
        let duplicate_terminated_bye = {
            let mut dialog = self.get_dialog_mut(&dialog_id)?;
            match classify_bye_sequence(&dialog, &request)? {
                ByeSequenceDisposition::Fresh => {
                    dialog.update_remote_sequence(&request)?;
                    dialog.terminate();
                    false
                }
                ByeSequenceDisposition::DuplicateTerminated => true,
            }
        };

        // Send 200 OK response
        let response = response_builders::create_response(&request, StatusCode::Ok);
        self.transaction_manager
            .send_response(&transaction_id, response)
            .await
            .map_err(|e| DialogError::TransactionError {
                message: format!("Failed to send 200 OK to BYE: {}", e),
            })?;
        self.release_bye_server_transaction(&transaction_id).await;

        if duplicate_terminated_bye {
            debug!(
                "Duplicate BYE processed idempotently for terminated dialog {}",
                dialog_id
            );
            return Ok(());
        }

        // Dialog-core has already sent the 200 OK, so keep the wire response
        // path independent from session-core cleanup fan-out under load.
        let event = SessionCoordinationEvent::ByeReceived {
            dialog_id: dialog_id.clone(),
        };
        let manager = self.clone();
        let cleanup_dialog_id = dialog_id.clone();
        tokio::spawn(async move {
            manager.emit_session_coordination_event(event).await;
            manager.remove_dialog_storage(&cleanup_dialog_id);
            debug!(
                "BYE cleanup event published for dialog {}",
                cleanup_dialog_id
            );
        });

        info!("BYE processed for dialog {}", dialog_id);
        Ok(())
    }

    async fn release_bye_server_transaction(&self, transaction_id: &TransactionKey) {
        if let Err(e) = self
            .transaction_manager
            .terminate_transaction(transaction_id)
            .await
        {
            warn!(
                "Failed to release completed BYE server transaction {}: {}",
                transaction_id, e
            );
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{ByeSequenceDisposition, classify_bye_sequence};
    use crate::dialog::{Dialog, DialogState};
    use rvoip_sip_core::builder::SimpleRequestBuilder;
    use rvoip_sip_core::{Method, Request};

    fn dialog_with_state(state: DialogState, remote_cseq: u32) -> Dialog {
        let mut dialog = Dialog::new(
            "bye-sequence-test".to_string(),
            "sip:alice@example.com".parse().unwrap(),
            "sip:bob@example.com".parse().unwrap(),
            Some("alice-tag".to_string()),
            Some("bob-tag".to_string()),
            true,
        );
        dialog.state = state;
        dialog.remote_cseq = remote_cseq;
        dialog
    }

    fn bye_request(cseq: u32) -> Request {
        SimpleRequestBuilder::new(Method::Bye, "sip:alice@example.com")
            .unwrap()
            .from("Bob", "sip:bob@example.com", Some("bob-tag"))
            .to("Alice", "sip:alice@example.com", Some("alice-tag"))
            .call_id("bye-sequence-test")
            .cseq(cseq)
            .via("127.0.0.1:5060", "UDP", Some("z9hG4bK-bye-sequence"))
            .max_forwards(70)
            .build()
    }

    #[test]
    fn duplicate_bye_on_terminated_dialog_is_idempotent() {
        let dialog = dialog_with_state(DialogState::Terminated, 2);
        let request = bye_request(2);

        assert_eq!(
            classify_bye_sequence(&dialog, &request).unwrap(),
            ByeSequenceDisposition::DuplicateTerminated
        );
    }

    #[test]
    fn same_cseq_bye_on_confirmed_dialog_remains_fresh_for_strict_validation() {
        let dialog = dialog_with_state(DialogState::Confirmed, 2);
        let request = bye_request(2);

        assert_eq!(
            classify_bye_sequence(&dialog, &request).unwrap(),
            ByeSequenceDisposition::Fresh
        );
    }

    #[test]
    fn missing_cseq_bye_still_fails_protocol_validation() {
        let dialog = dialog_with_state(DialogState::Terminated, 2);
        let request = Request::new(Method::Bye, "sip:alice@example.com".parse().unwrap());

        assert!(classify_bye_sequence(&dialog, &request).is_err());
    }
}
