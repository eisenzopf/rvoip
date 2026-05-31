//! `UpdateBuilder` — SIP_API_DESIGN_2 §3.3 (RFC 3311 UPDATE).

use std::sync::Arc;

use rvoip_sip_core::types::Method;

use crate::api::handle::CallId;
use crate::api::headers::{take_staged, BuilderHeaderState, SipRequestOptions};
use crate::api::unified::UnifiedCoordinator;
use crate::errors::Result;

/// In-dialog UPDATE builder (RFC 3311). Reachable via
/// [`UnifiedCoordinator::update`](crate::api::unified::UnifiedCoordinator::update).
pub struct UpdateBuilder {
    coord: Arc<UnifiedCoordinator>,
    session_id: CallId,
    sdp: Option<String>,
    session_timer_refresh: bool,
    state: BuilderHeaderState,
}

impl UpdateBuilder {
    pub(crate) fn new(coord: Arc<UnifiedCoordinator>, session_id: CallId) -> Self {
        Self {
            coord,
            session_id,
            sdp: None,
            session_timer_refresh: false,
            state: BuilderHeaderState::default(),
        }
    }

    /// Attach the renegotiated SDP offer.
    pub fn with_sdp(mut self, sdp: impl Into<String>) -> Self {
        self.sdp = Some(sdp.into());
        self
    }
    /// Mark this UPDATE as an RFC 4028 session-timer refresh.
    pub fn as_session_timer_refresh(mut self) -> Self {
        self.session_timer_refresh = true;
        self
    }

    /// Send the UPDATE through the dialog's state machine.
    pub async fn send(mut self) -> Result<()> {
        let extra_headers = take_staged(&mut self.state);
        let opts = Arc::new(rvoip_sip_dialog::api::unified::UpdateRequestOptions {
            sdp: self.sdp,
            session_timer_refresh: self.session_timer_refresh,
            extra_headers,
        });
        self.coord
            .stage_outbound_options(
                &self.session_id,
                crate::state_machine::executor::PendingOptionsSlot::Update(opts),
            )
            .await?;
        self.coord
            .dispatch_outbound(
                &self.session_id,
                crate::state_table::EventType::SendOutboundUpdate,
            )
            .await?;
        Ok(())
    }
}

impl SipRequestOptions for UpdateBuilder {
    fn method(&self) -> Method {
        Method::Update
    }
    fn header_state_mut(&mut self) -> &mut BuilderHeaderState {
        &mut self.state
    }
    fn header_state(&self) -> &BuilderHeaderState {
        &self.state
    }
}
