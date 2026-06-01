//! `ProvisionalBuilder` — SIP_API_DESIGN_2 §3.4.

use std::sync::Arc;

use rvoip_sip_core::types::Method;

use crate::api::handle::CallId;
use crate::api::headers::{take_staged, BuilderHeaderState, SipRequestOptions};
use crate::api::unified::UnifiedCoordinator;
use crate::errors::Result;
use rvoip_sip_core::types::headers::{HeaderName, HeaderValue, TypedHeader};

/// Builds and sends a 1xx provisional response (e.g. 180 Ringing,
/// 183 Session Progress) for an inbound INVITE.
pub struct ProvisionalBuilder {
    coord: Arc<UnifiedCoordinator>,
    call_id: CallId,
    code: u16,
    sdp: Option<String>,
    require_100rel: bool,
    state: BuilderHeaderState,
}

impl ProvisionalBuilder {
    pub(crate) fn new(coord: Arc<UnifiedCoordinator>, call_id: CallId, code: u16) -> Self {
        Self {
            coord,
            call_id,
            code,
            sdp: None,
            require_100rel: false,
            state: BuilderHeaderState::default(),
        }
    }

    /// Set the early-media SDP for the provisional message body.
    pub fn with_sdp(mut self, sdp: impl Into<String>) -> Self {
        self.sdp = Some(sdp.into());
        self
    }
    /// Require reliable provisional delivery, stamping `Require: 100rel`
    /// (RFC 3262) on the response.
    pub fn with_require_100rel(mut self, require: bool) -> Self {
        self.require_100rel = require;
        self
    }

    /// Send the provisional response on the wire.
    pub async fn send(mut self) -> Result<()> {
        let mut extras = take_staged(&mut self.state);

        // Per design §3.3 setter table, `with_require_100rel(true)`
        // stamps `Require: 100rel` on the provisional. The matching
        // RSeq is set by the state machine on emission.
        if self.require_100rel {
            extras.push(TypedHeader::Other(
                HeaderName::Require,
                HeaderValue::Raw(b"100rel".to_vec()),
            ));
        }

        // The legacy send_early_media path always emits 183 with the
        // reliability bits driven by Config + peer Supported, so when
        // no extras are staged and the requested code matches the
        // legacy default, keep that path for backwards-compatibility.
        if extras.is_empty() && (self.code == 183 || self.code == 180) {
            return self.coord.send_early_media(&self.call_id, self.sdp).await;
        }

        // SIP_API_DESIGN_2 Phase D: extras-aware provisional dispatch.
        // Routes through `send_response_with_options` so 100rel /
        // Server / Allow staging from the upstream leg ride to the
        // downstream wire intact.
        self.coord
            .dialog_adapter()
            .send_response_with_options(&self.call_id, self.code, self.sdp, extras)
            .await
    }
}

impl SipRequestOptions for ProvisionalBuilder {
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
