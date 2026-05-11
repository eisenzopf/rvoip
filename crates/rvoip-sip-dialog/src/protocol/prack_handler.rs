//! PRACK Request Handler for Dialog-Core
//!
//! Handles incoming PRACK requests per RFC 3262 §7.2. PRACK acknowledges a
//! reliable provisional response and must match an unacknowledged reliable
//! provisional by its `RAck` header. On a match we stop retransmitting the
//! 18x and reply 200 OK; on no match we reply 481.

use tracing::{debug, warn};

use rvoip_sip_core::types::rack::RAck;
use rvoip_sip_core::types::TypedHeader;
use rvoip_sip_core::{Request, StatusCode};

use crate::errors::{DialogError, DialogResult};
use crate::manager::{DialogManager, SourceExtractor};
use crate::transaction::utils::response_builders;

/// PRACK-specific handling operations
pub trait PrackHandler {
    /// Handle incoming PRACK requests (RFC 3262).
    fn handle_prack_method(
        &self,
        request: Request,
    ) -> impl std::future::Future<Output = DialogResult<()>> + Send;
}

impl PrackHandler for DialogManager {
    async fn handle_prack_method(&self, request: Request) -> DialogResult<()> {
        debug!("Processing PRACK request");

        let rack = request.headers.iter().find_map(|h| {
            if let TypedHeader::RAck(r) = h {
                Some(r.clone())
            } else {
                None
            }
        });

        let source = SourceExtractor::extract_from_request(&request);
        let server_tx = self
            .transaction_manager
            .create_server_transaction(request.clone(), source)
            .await
            .map_err(|e| DialogError::TransactionError {
                message: format!("Failed to create server transaction for PRACK: {}", e),
            })?;
        let transaction_id = server_tx.id().clone();

        let Some(rack) = rack else {
            warn!("PRACK missing RAck header — sending 400 Bad Request");
            let response = response_builders::create_response(&request, StatusCode::BadRequest);
            let _ = self
                .transaction_manager
                .send_response(&transaction_id, response)
                .await;
            return Ok(());
        };

        let dialog_id = match self.find_dialog_for_request(&request).await {
            Some(id) => id,
            None => {
                debug!("PRACK: no dialog found — sending 481");
                let response = response_builders::create_response(
                    &request,
                    StatusCode::CallOrTransactionDoesNotExist,
                );
                let _ = self
                    .transaction_manager
                    .send_response(&transaction_id, response)
                    .await;
                return Ok(());
            }
        };

        if !self.try_ack_reliable_provisional(&dialog_id, &rack) {
            debug!(
                "PRACK with RAck {} {} {} does not match any unacked reliable provisional — sending 481",
                rack.rseq, rack.cseq, rack.method
            );
            let response = response_builders::create_response(
                &request,
                StatusCode::CallOrTransactionDoesNotExist,
            );
            self.transaction_manager
                .send_response(&transaction_id, response)
                .await
                .map_err(|e| DialogError::TransactionError {
                    message: format!("Failed to send 481 to spurious PRACK: {}", e),
                })?;
            return Ok(());
        }

        {
            let mut dialog = self.get_dialog_mut(&dialog_id)?;
            dialog.update_remote_sequence(&request).ok();
        }

        let response = response_builders::create_response(&request, StatusCode::Ok);
        self.transaction_manager
            .send_response(&transaction_id, response)
            .await
            .map_err(|e| DialogError::TransactionError {
                message: format!("Failed to send 200 OK to PRACK: {}", e),
            })?;

        debug!(
            "PRACK acknowledged reliable 18x (dialog {}, rseq {})",
            dialog_id, rack.rseq
        );
        Ok(())
    }
}

impl DialogManager {
    /// Cancel the retransmit task for a matching reliable provisional.
    /// Returns `true` when an outstanding provisional was found and aborted.
    ///
    /// The `RAck.cseq` should equal the dialog's stored `invite_cseq`; if it
    /// doesn't we still look up by `rseq` alone because CSeq match on the
    /// dialog is implicit (only one INVITE per dialog from a given side).
    pub(crate) fn try_ack_reliable_provisional(
        &self,
        dialog_id: &crate::dialog::DialogId,
        rack: &RAck,
    ) -> bool {
        let key = (dialog_id.clone(), rack.rseq);
        let Some((_, abort)) = self.reliable_provisional_tasks.remove(&key) else {
            return false;
        };
        abort.abort();
        true
    }
}
