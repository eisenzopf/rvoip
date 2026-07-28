//! `ProvisionalBuilder` — SIP_API_DESIGN_2 §3.4.

use std::sync::Arc;

use rvoip_sip_core::types::Method;

use crate::api::handle::CallId;
use crate::api::headers::{take_staged, BuilderHeaderState, SipRequestOptions};
use crate::api::unified::UnifiedCoordinator;
use crate::errors::{Result, SessionError};
use crate::session_registry::SessionRegistryHandle;
use rvoip_sip_core::types::headers::{HeaderName, HeaderValue, TypedHeader};

/// Builds and sends a 1xx provisional response (e.g. 180 Ringing,
/// 183 Session Progress) for an inbound INVITE.
pub struct ProvisionalBuilder {
    coord: Arc<UnifiedCoordinator>,
    call_id: CallId,
    lifecycle_handle: Option<SessionRegistryHandle>,
    code: u16,
    sdp: Option<String>,
    require_100rel: bool,
    state: BuilderHeaderState,
}

impl ProvisionalBuilder {
    pub(crate) fn new(coord: Arc<UnifiedCoordinator>, call_id: CallId, code: u16) -> Self {
        let lifecycle_handle = coord.helpers.state_machine.store.lifecycle_handle(&call_id);
        Self::new_captured(coord, call_id, lifecycle_handle, code)
    }

    pub(crate) fn new_captured(
        coord: Arc<UnifiedCoordinator>,
        call_id: CallId,
        lifecycle_handle: Option<SessionRegistryHandle>,
        code: u16,
    ) -> Self {
        Self {
            coord,
            call_id,
            lifecycle_handle,
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
        let lifecycle_handle = self
            .lifecycle_handle
            .as_ref()
            .ok_or_else(|| SessionError::SessionNotFound(self.call_id.to_string()))?;
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

        // Preserve the compatibility path's peer-capability rejection for
        // ordinary 180/183 sends. Dispatch itself still enters through the
        // captured exact lifecycle below.
        if extras.is_empty() && (self.code == 183 || self.code == 180) {
            if !self
                .coord
                .dialog_adapter()
                .peer_supports_100rel(&self.call_id)
                .await?
            {
                return Err(crate::errors::SessionError::UnreliableProvisionalsNotSupported);
            }
        }

        // The requested 1xx, body, and headers enter the exact-session lane as
        // one response envelope. The YAML SendEarlyMedia transition remains
        // the lifecycle/action-order authority and dialog-core owns RSeq.
        self.coord
            .helpers
            .send_provisional_with_response(
                &self.call_id,
                lifecycle_handle,
                self.code,
                self.sdp,
                extras,
            )
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
