//! `NotifyBuilder` — SIP_API_DESIGN_2 §3.3.

use std::sync::Arc;

use bytes::Bytes;
use rvoip_sip_core::types::Method;

use crate::api::handle::CallId;
use crate::api::headers::{take_staged, BuilderHeaderState, SipRequestOptions};
use crate::api::unified::UnifiedCoordinator;
use crate::errors::Result;

pub struct NotifyBuilder {
    coord: Arc<UnifiedCoordinator>,
    session_id: CallId,
    event_package: String,
    body: Option<Bytes>,
    content_type: Option<String>,
    subscription_state: Option<String>,
    retry_after: Option<u32>,
    subscription_id: Option<String>,
    state: BuilderHeaderState,
}

impl NotifyBuilder {
    pub(crate) fn new(
        coord: Arc<UnifiedCoordinator>,
        session_id: CallId,
        event_package: impl Into<String>,
    ) -> Self {
        Self {
            coord,
            session_id,
            event_package: event_package.into(),
            body: None,
            content_type: None,
            subscription_state: None,
            retry_after: None,
            subscription_id: None,
            state: BuilderHeaderState::default(),
        }
    }

    pub fn with_body(mut self, body: impl Into<Bytes>) -> Self {
        self.body = Some(body.into());
        self
    }
    pub fn with_content_type(mut self, ct: impl Into<String>) -> Self {
        self.content_type = Some(ct.into());
        self
    }
    pub fn with_subscription_state(mut self, s: impl Into<String>) -> Self {
        self.subscription_state = Some(s.into());
        self
    }
    pub fn with_retry_after(mut self, seconds: u32) -> Self {
        self.retry_after = Some(seconds);
        self
    }
    pub fn for_subscription(mut self, id: impl Into<String>) -> Self {
        self.subscription_id = Some(id.into());
        self
    }

    pub async fn send(mut self) -> Result<()> {
        let extra_headers = take_staged(&mut self.state);
        let opts = Arc::new(rvoip_sip_dialog::api::unified::NotifyRequestOptions {
            event: self.event_package,
            subscription_state: self.subscription_state.unwrap_or_default(),
            content_type: self.content_type,
            body: self.body,
            subscription_id: self.subscription_id,
            extra_headers,
        });
        // retry_after is staged on the builder but dialog-core's
        // options surface doesn't carry it; it would land on the wire
        // via with_raw_header in the meantime.
        let _ = self.retry_after;
        self.coord
            .stage_outbound_options(
                &self.session_id,
                crate::state_machine::executor::PendingOptionsSlot::Notify(opts),
            )
            .await?;
        self.coord
            .dispatch_outbound(
                &self.session_id,
                crate::state_table::EventType::SendOutboundNotify,
            )
            .await?;
        Ok(())
    }
}

impl SipRequestOptions for NotifyBuilder {
    fn method(&self) -> Method {
        Method::Notify
    }
    fn header_state_mut(&mut self) -> &mut BuilderHeaderState {
        &mut self.state
    }
    fn header_state(&self) -> &BuilderHeaderState {
        &self.state
    }
}
