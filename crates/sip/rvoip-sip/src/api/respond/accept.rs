//! `AcceptBuilder` — SIP_API_DESIGN_2 §3.4.

use std::sync::Arc;

use rvoip_sip_core::types::Method;

use crate::api::handle::{CallId, SessionHandle};
use crate::api::headers::{take_staged, BuilderHeaderState, SipRequestOptions};
use crate::api::unified::UnifiedCoordinator;
use crate::errors::{Result, SessionError};
use crate::session_registry::SessionRegistryHandle;

/// Builds and sends a 200 OK accepting an inbound INVITE.
pub struct AcceptBuilder {
    coord: Arc<UnifiedCoordinator>,
    call_id: CallId,
    lifecycle_handle: Option<SessionRegistryHandle>,
    sdp: Option<String>,
    state: BuilderHeaderState,
}

impl AcceptBuilder {
    pub(crate) fn new(coord: Arc<UnifiedCoordinator>, call_id: CallId) -> Self {
        let lifecycle_handle = coord.helpers.state_machine.store.lifecycle_handle(&call_id);
        Self::new_captured(coord, call_id, lifecycle_handle)
    }

    pub(crate) fn new_captured(
        coord: Arc<UnifiedCoordinator>,
        call_id: CallId,
        lifecycle_handle: Option<SessionRegistryHandle>,
    ) -> Self {
        Self {
            coord,
            call_id,
            lifecycle_handle,
            sdp: None,
            state: BuilderHeaderState::default(),
        }
    }

    /// Set the answer SDP for the 200 OK message body.
    pub fn with_sdp(mut self, sdp: impl Into<String>) -> Self {
        self.sdp = Some(sdp.into());
        self
    }

    /// Send the 200 OK and return a handle to the now-established session.
    pub async fn send(mut self) -> Result<SessionHandle> {
        let lifecycle_handle = self
            .lifecycle_handle
            .clone()
            .ok_or_else(|| SessionError::SessionNotFound(self.call_id.to_string()))?;
        if self.coord.fast_auto_accept_incoming_calls() {
            return Ok(SessionHandle::new_exact(
                self.call_id,
                self.coord,
                lifecycle_handle,
            ));
        }

        let extras = take_staged(&mut self.state);
        self.coord
            .accept_call_with_response(&self.call_id, &lifecycle_handle, self.sdp, extras)
            .await?;

        Ok(SessionHandle::new_exact(
            self.call_id,
            self.coord,
            lifecycle_handle,
        ))
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
