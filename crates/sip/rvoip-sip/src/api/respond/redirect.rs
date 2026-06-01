//! `RedirectBuilder` — SIP_API_DESIGN_2 §3.4.

use std::sync::Arc;

use rvoip_sip_core::types::Method;

use crate::api::handle::CallId;
use crate::api::headers::{take_staged, BuilderHeaderState, SipRequestOptions};
use crate::api::unified::UnifiedCoordinator;
use crate::errors::Result;

/// Builds and sends a 3xx redirect response (default 302 Moved
/// Temporarily) carrying one or more `Contact:` targets.
pub struct RedirectBuilder {
    coord: Arc<UnifiedCoordinator>,
    call_id: CallId,
    status: u16,
    contacts: Vec<String>,
    state: BuilderHeaderState,
}

impl RedirectBuilder {
    pub(crate) fn new(coord: Arc<UnifiedCoordinator>, call_id: CallId) -> Self {
        Self {
            coord,
            call_id,
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
        if self.coord.fast_auto_accept_incoming_calls() {
            return Ok(());
        }

        let extras = take_staged(&mut self.state);
        if extras.is_empty() {
            return self
                .coord
                .helpers
                .redirect_call(&self.call_id, self.status, self.contacts)
                .await;
        }
        // SIP_API_DESIGN_2 Phase D: route through the extras-aware
        // redirect path so staged headers (e.g., Retry-After on a 305)
        // ride to the wire.
        self.coord
            .dialog_adapter()
            .send_redirect_response_with_options(&self.call_id, self.status, self.contacts, extras)
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
