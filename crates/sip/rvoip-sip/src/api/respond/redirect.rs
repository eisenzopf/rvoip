//! `RedirectBuilder` — SIP_API_DESIGN_2 §3.4.

use std::sync::Arc;

use rvoip_sip_core::types::Method;

use crate::api::handle::CallId;
use crate::api::headers::{take_staged, BuilderHeaderState, SipRequestOptions};
use crate::api::unified::UnifiedCoordinator;
use crate::errors::{Result, SessionError};
use crate::session_registry::SessionRegistryHandle;

/// Builds and sends a 3xx redirect response (default 302 Moved
/// Temporarily) carrying one or more `Contact:` targets.
pub struct RedirectBuilder {
    coord: Arc<UnifiedCoordinator>,
    call_id: CallId,
    lifecycle_handle: Option<SessionRegistryHandle>,
    status: u16,
    contacts: Vec<String>,
    state: BuilderHeaderState,
}

impl RedirectBuilder {
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
            status: 302,
            contacts: Vec::new(),
            state: BuilderHeaderState::default(),
        }
    }

    /// Set the 3xx status code (e.g. 301, 302, 305).
    pub fn with_status(mut self, code: u16) -> Self {
        self.status = code;
        self
    }
    /// Append a single redirect target (`Contact:` URI).
    pub fn with_contact(mut self, uri: impl Into<String>) -> Self {
        self.contacts.push(uri.into());
        self
    }
    /// Append multiple redirect targets (`Contact:` URIs).
    pub fn with_contacts(mut self, uris: Vec<String>) -> Self {
        self.contacts.extend(uris);
        self
    }

    /// Send the redirect response on the wire.
    pub async fn send(mut self) -> Result<()> {
        let lifecycle_handle = self
            .lifecycle_handle
            .as_ref()
            .ok_or_else(|| SessionError::SessionNotFound(self.call_id.to_string()))?;
        if self.coord.fast_auto_accept_incoming_calls() {
            return Ok(());
        }

        let extras = take_staged(&mut self.state);
        self.coord
            .helpers
            .redirect_call_with_extras_exact(lifecycle_handle, self.status, self.contacts, extras)
            .await
    }
}

impl SipRequestOptions for RedirectBuilder {
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
