//! `GenericResponseBuilder` — SIP_API_DESIGN_2 §3.4.

use std::sync::Arc;

use rvoip_sip_core::types::Method;
use rvoip_sip_dialog::transaction::TransactionKey;

use crate::api::handle::CallId;
use crate::api::headers::{take_staged, BuilderHeaderState, SipRequestOptions};
use crate::api::incoming::ExactResponseObligation;
use crate::api::unified::UnifiedCoordinator;
use crate::errors::{Result, SessionError};

/// Builds and sends a generic non-2xx final response (3xx/4xx/5xx/6xx).
pub struct GenericResponseBuilder {
    coord: Arc<UnifiedCoordinator>,
    call_id: CallId,
    method: Method,
    status: u16,
    reason: Option<String>,
    exact_transaction: Option<TransactionKey>,
    response_obligation: Option<Arc<ExactResponseObligation>>,
    state: BuilderHeaderState,
}

impl GenericResponseBuilder {
    /// `method` is the request method this response is for — drives
    /// `HeaderPolicy::classify` so attaching e.g. `Event:` on a
    /// NOTIFY-shaped `respond_builder(401)` raises the appropriate
    /// dedicated-setter error. Per SIP_API_DESIGN_2 §3.4 every
    /// response builder must thread the inbound method through.
    pub(crate) fn new(
        coord: Arc<UnifiedCoordinator>,
        call_id: CallId,
        method: Method,
        status: u16,
    ) -> Result<Self> {
        // Status range guard per §3.4: 3xx/4xx/5xx/6xx only.
        if !(300..=699).contains(&status) {
            return Err(SessionError::InvalidInput(format!(
                "GenericResponseBuilder status must be 3xx/4xx/5xx/6xx, got {status}"
            )));
        }
        Ok(Self {
            coord,
            call_id,
            method,
            status,
            reason: None,
            exact_transaction: None,
            response_obligation: None,
            state: BuilderHeaderState::default(),
        })
    }

    pub(crate) fn new_in_dialog(
        coord: Arc<UnifiedCoordinator>,
        call_id: CallId,
        method: Method,
        transaction_id: TransactionKey,
        status: u16,
        response_obligation: Arc<ExactResponseObligation>,
    ) -> Result<Self> {
        if !(200..=699).contains(&status) {
            return Err(SessionError::InvalidInput(format!(
                "exact in-dialog response status must be 2xx/3xx/4xx/5xx/6xx, got {status}"
            )));
        }
        if !transaction_id.is_server()
            || transaction_id.method() != &method
            || method == Method::Invite
        {
            return Err(SessionError::InvalidInput(
                "exact in-dialog response requires the matching non-INVITE server transaction"
                    .to_string(),
            ));
        }
        Ok(Self {
            coord,
            call_id,
            method,
            status,
            reason: None,
            exact_transaction: Some(transaction_id),
            response_obligation: Some(response_obligation),
            state: BuilderHeaderState::default(),
        })
    }

    /// Set the response reason phrase (defaults to a status-derived value).
    pub fn with_reason(mut self, r: impl Into<String>) -> Self {
        self.reason = Some(r.into());
        self
    }

    /// Send the response, routing 3xx through the redirect path and
    /// 4xx/5xx/6xx through the reject path.
    pub async fn send(mut self) -> Result<()> {
        let reason = self.reason.unwrap_or_else(|| "OK".to_string());
        let extras = take_staged(&mut self.state);

        if let Some(transaction_id) = self.exact_transaction.take() {
            let obligation = self.response_obligation.take().ok_or_else(|| {
                SessionError::InternalError(
                    "exact in-dialog response has no response obligation".to_string(),
                )
            })?;
            let claim = obligation.claim()?;
            let result = self
                .coord
                .dialog_adapter()
                .send_response_with_options_for_transaction_classified(
                    &self.call_id,
                    &transaction_id,
                    self.status,
                    None,
                    extras,
                )
                .await;
            return match result {
                Ok(rvoip_sip_dialog::FinalResponseCompletionDisposition::WrittenSuccessTerminal) => {
                    claim.complete();
                    Ok(())
                }
                Ok(disposition) => {
                    claim.complete();
                    Err(SessionError::InternalError(format!(
                        "exact response returned nonterminal success disposition: {disposition:?}"
                    )))
                }
                Err(error)
                    if error.disposition
                        == rvoip_sip_dialog::FinalResponseCompletionDisposition::ZeroWireRetryable =>
                {
                    claim.release_after_failure();
                    Err(SessionError::DialogError(format!(
                        "Failed to send exact in-dialog response: {}",
                        error.source
                    )))
                }
                Err(error) => {
                    claim.complete();
                    Err(SessionError::DialogError(format!(
                        "Exact in-dialog response became wire-unknown and will not be retried: {}",
                        error.source
                    )))
                }
            };
        }

        // 3xx → redirect path; 4xx/5xx/6xx → reject path.
        if (300..=399).contains(&self.status) {
            if extras.is_empty() {
                self.coord
                    .helpers
                    .redirect_call(&self.call_id, self.status, vec![reason])
                    .await
            } else {
                self.coord
                    .dialog_adapter()
                    .send_redirect_response_with_options(
                        &self.call_id,
                        self.status,
                        vec![reason],
                        extras,
                    )
                    .await
            }
        } else if extras.is_empty() {
            self.coord
                .helpers
                .reject_call(&self.call_id, self.status, &reason)
                .await
        } else {
            self.coord
                .dialog_adapter()
                .send_response_with_options(&self.call_id, self.status, None, extras)
                .await?;
            // Mirror the legacy reject path's state-machine teardown
            // so the session settles to the correct terminal state.
            self.coord
                .helpers
                .reject_call(&self.call_id, self.status, &reason)
                .await
                .or(Ok(()))
        }
    }
}

impl SipRequestOptions for GenericResponseBuilder {
    fn method(&self) -> Method {
        // Returns the request method threaded through the constructor,
        // not a hardcoded INVITE — SIP_API_DESIGN_2 §3.4 requires the
        // response builder's policy classification to track the
        // underlying request method.
        self.method.clone()
    }
    fn header_state_mut(&mut self) -> &mut BuilderHeaderState {
        &mut self.state
    }
    fn header_state(&self) -> &BuilderHeaderState {
        &self.state
    }
}
