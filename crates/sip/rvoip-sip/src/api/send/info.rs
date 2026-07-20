//! `InfoBuilder` — SIP_API_DESIGN_2 §3.3.

use std::sync::Arc;

use bytes::Bytes;
use rvoip_sip_core::types::Method;

use crate::api::handle::CallId;
use crate::api::headers::{take_staged, BuilderHeaderState, SipRequestOptions};
use crate::api::unified::UnifiedCoordinator;
use crate::errors::Result;

/// In-dialog INFO builder (RFC 6086). Reachable via
/// [`UnifiedCoordinator::info`](crate::api::unified::UnifiedCoordinator::info).
pub struct InfoBuilder {
    coord: Arc<UnifiedCoordinator>,
    session_id: CallId,
    content_type: String,
    body: Option<Bytes>,
    state: BuilderHeaderState,
}

impl InfoBuilder {
    pub(crate) fn new(
        coord: Arc<UnifiedCoordinator>,
        session_id: CallId,
        content_type: impl Into<String>,
    ) -> Self {
        Self {
            coord,
            session_id,
            content_type: content_type.into(),
            body: None,
            state: BuilderHeaderState::default(),
        }
    }

    /// Attach the INFO request body.
    pub fn with_body(mut self, body: impl Into<Bytes>) -> Self {
        self.body = Some(body.into());
        self
    }

    /// Send the INFO through the dialog's state machine.
    pub async fn send(mut self) -> Result<()> {
        let body = self.body.unwrap_or_default();
        let extra_headers = take_staged(&mut self.state);
        let opts = Arc::new(rvoip_sip_dialog::api::unified::InfoRequestOptions {
            content_type: self.content_type,
            body,
            extra_headers,
        });
        let staging = self
            .coord
            .stage_outbound_options_guarded(
                &self.session_id,
                crate::state_machine::executor::PendingOptionsSlot::Info(opts),
            )
            .await?;
        self.coord
            .dispatch_outbound_guarded(
                &self.session_id,
                crate::state_table::EventType::SendOutboundInfo,
                &staging,
            )
            .await?;
        staging.confirm_consumed().await?;
        Ok(())
    }
}

impl SipRequestOptions for InfoBuilder {
    fn method(&self) -> Method {
        Method::Info
    }
    fn header_state_mut(&mut self) -> &mut BuilderHeaderState {
        &mut self.state
    }
    fn header_state(&self) -> &BuilderHeaderState {
        &self.state
    }
}
