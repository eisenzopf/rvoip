//! `GenericResponseBuilder` — SIP_API_DESIGN_2 §3.4.

use std::sync::Arc;

use rvoip_sip_core::types::Method;

use crate::api::handle::CallId;
use crate::api::headers::{take_staged, BuilderHeaderState, SipRequestOptions};
use crate::api::unified::UnifiedCoordinator;
use crate::errors::{Result, SessionError};

pub struct GenericResponseBuilder {
    coord: Arc<UnifiedCoordinator>,
    call_id: CallId,
    method: Method,
    status: u16,
    reason: Option<String>,
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
            state: BuilderHeaderState::default(),
        })
    }

    pub fn with_reason(mut self, r: impl Into<String>) -> Self {
        self.reason = Some(r.into());
        self
    }

    pub async fn send(mut self) -> Result<()> {
        let reason = self.reason.unwrap_or_else(|| "OK".to_string());
        let extras = take_staged(&mut self.state);

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
