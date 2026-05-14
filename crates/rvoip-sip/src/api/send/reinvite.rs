//! `ReInviteBuilder` — SIP_API_DESIGN_2 §3.3.

use std::sync::Arc;

use rvoip_sip_core::types::Method;

use crate::api::handle::CallId;
use crate::api::headers::{take_staged, BuilderHeaderState, SipRequestOptions};
use crate::api::unified::UnifiedCoordinator;
use crate::errors::Result;

pub struct ReInviteBuilder {
    coord: Arc<UnifiedCoordinator>,
    session_id: CallId,
    sdp: Option<String>,
    session_timer_refresh: bool,
    precomputed_authorization: Option<String>,
    state: BuilderHeaderState,
}

impl ReInviteBuilder {
    pub(crate) fn new(coord: Arc<UnifiedCoordinator>, session_id: CallId) -> Self {
        Self {
            coord,
            session_id,
            sdp: None,
            session_timer_refresh: false,
            precomputed_authorization: None,
            state: BuilderHeaderState::default(),
        }
    }

    pub fn with_sdp(mut self, sdp: impl Into<String>) -> Self {
        self.sdp = Some(sdp.into());
        self
    }
    pub fn as_session_timer_refresh(mut self) -> Self {
        self.session_timer_refresh = true;
        self
    }
    pub fn with_precomputed_authorization(mut self, s: impl Into<String>) -> Self {
        self.precomputed_authorization = Some(s.into());
        self
    }

    pub async fn send(mut self) -> Result<()> {
        let extra_headers = take_staged(&mut self.state);
        let opts = Arc::new(rvoip_sip_dialog::api::unified::ReInviteRequestOptions {
            sdp: self.sdp,
            session_timer_refresh: self.session_timer_refresh,
            precomputed_authorization: self.precomputed_authorization,
            extra_headers,
        });
        self.coord
            .stage_outbound_options(
                &self.session_id,
                crate::state_machine::executor::PendingOptionsSlot::ReInvite(opts),
            )
            .await?;
        self.coord
            .dispatch_outbound(
                &self.session_id,
                crate::state_table::EventType::SendOutboundReInvite,
            )
            .await?;
        Ok(())
    }
}

impl SipRequestOptions for ReInviteBuilder {
    fn method(&self) -> Method {
        Method::Invite
    }
    fn header_state_mut(&mut self) -> &mut BuilderHeaderState {
        &mut self.state
    }
    fn header_state(&self) -> &BuilderHeaderState {
        &self.state
    }
}
