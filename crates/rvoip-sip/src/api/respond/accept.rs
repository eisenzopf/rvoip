//! `AcceptBuilder` — SIP_API_DESIGN_2 §3.4.

use std::sync::Arc;

use rvoip_sip_core::types::Method;

use crate::api::handle::{CallId, SessionHandle};
use crate::api::headers::{take_staged, BuilderHeaderState, SipRequestOptions};
use crate::api::unified::UnifiedCoordinator;
use crate::errors::Result;

pub struct AcceptBuilder {
    coord: Arc<UnifiedCoordinator>,
    call_id: CallId,
    sdp: Option<String>,
    state: BuilderHeaderState,
}

impl AcceptBuilder {
    pub(crate) fn new(coord: Arc<UnifiedCoordinator>, call_id: CallId) -> Self {
        Self {
            coord,
            call_id,
            sdp: None,
            state: BuilderHeaderState::default(),
        }
    }

    pub fn with_sdp(mut self, sdp: impl Into<String>) -> Self {
        self.sdp = Some(sdp.into());
        self
    }

    pub async fn send(mut self) -> Result<SessionHandle> {
        if self.coord.fast_auto_accept_incoming_calls() {
            return Ok(SessionHandle::new(self.call_id, self.coord));
        }

        let extras = take_staged(&mut self.state);
        if extras.is_empty() {
            // Preserve the legacy media-negotiating path when the
            // application has not staged any extras — the legacy
            // accept_call_with_sdp / accept_call entries handle local
            // SDP synthesis for the no-SDP case.
            match self.sdp {
                Some(sdp) => self.coord.accept_call_with_sdp(&self.call_id, sdp).await?,
                None => self.coord.accept_call(&self.call_id).await?,
            }
        } else {
            // SIP_API_DESIGN_2 Phase D: route through the
            // extras-aware response path so staged headers reach the
            // wire after stack-managed headers are stamped.
            self.coord
                .dialog_adapter()
                .send_response_with_options(&self.call_id, 200, self.sdp, extras)
                .await?;
        }
        Ok(SessionHandle::new(self.call_id, self.coord))
    }
}

impl SipRequestOptions for AcceptBuilder {
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
