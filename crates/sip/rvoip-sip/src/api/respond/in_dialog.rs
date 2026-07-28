//! Exact final responses for inbound in-dialog requests.

use std::sync::Arc;

use rvoip_sip_dialog::transaction::TransactionKey;

use crate::api::handle::CallId;
use crate::api::incoming::ExactResponseObligation;
use crate::api::unified::UnifiedCoordinator;
use crate::errors::{Result, SessionError};

/// Builds a final response for one exact inbound in-dialog transaction.
///
/// Unlike INVITE accept/reject builders, sending this response does not
/// transition or terminate the call. It is intended for application-owned
/// INFO and other non-INVITE requests whose result must be correlated to the
/// precise server transaction that delivered them.
pub struct InDialogResponseBuilder {
    inner: crate::api::respond::GenericResponseBuilder,
}

impl InDialogResponseBuilder {
    pub(crate) fn new(
        coord: Arc<UnifiedCoordinator>,
        call_id: CallId,
        transaction_id: TransactionKey,
        status: u16,
        response_obligation: Arc<ExactResponseObligation>,
    ) -> Result<Self> {
        let method = transaction_id.method().clone();
        validate_exact_final_response(&transaction_id, status)?;
        Ok(Self {
            inner: crate::api::respond::GenericResponseBuilder::new_in_dialog(
                coord,
                call_id,
                method,
                transaction_id,
                status,
                response_obligation,
            )?,
        })
    }

    /// Send the final response without changing the surrounding call state.
    pub async fn send(self) -> Result<()> {
        self.inner.send().await
    }
}

fn validate_exact_final_response(transaction_id: &TransactionKey, status: u16) -> Result<()> {
    if !(200..=699).contains(&status) {
        return Err(SessionError::InvalidInput(format!(
            "InDialogResponseBuilder status must be 2xx/3xx/4xx/5xx/6xx, got {status}"
        )));
    }
    if !transaction_id.is_server() || transaction_id.method() == &rvoip_sip_core::Method::Invite {
        return Err(SessionError::InvalidInput(
            "InDialogResponseBuilder requires an exact non-INVITE server transaction".to_string(),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;
    use rvoip_sip_core::Method;

    fn transaction(method: Method, is_server: bool) -> TransactionKey {
        TransactionKey::new("z9hG4bK-exact-response-test".to_string(), method, is_server)
    }

    #[test]
    fn accepts_only_final_response_status_range() {
        let info = transaction(Method::Info, true);
        assert!(validate_exact_final_response(&info, 200).is_ok());
        assert!(validate_exact_final_response(&info, 699).is_ok());
        assert!(validate_exact_final_response(&info, 199).is_err());
        assert!(validate_exact_final_response(&info, 700).is_err());
    }

    #[test]
    fn requires_exact_non_invite_server_transaction() {
        assert!(validate_exact_final_response(&transaction(Method::Info, true), 200).is_ok());
        assert!(validate_exact_final_response(&transaction(Method::Info, false), 200).is_err());
        assert!(validate_exact_final_response(&transaction(Method::Invite, true), 200).is_err());
    }
}
