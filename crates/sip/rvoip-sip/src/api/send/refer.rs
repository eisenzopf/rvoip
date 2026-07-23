//! `ReferBuilder` — SIP_API_DESIGN_2 §3.3.

use std::sync::Arc;

use rvoip_sip_core::types::Method;

use crate::api::handle::CallId;
use crate::api::headers::{take_staged, BuilderHeaderState, SipRequestOptions};
use crate::api::unified::UnifiedCoordinator;
use crate::errors::Result;
use crate::session_registry::SessionRegistryHandle;

use super::InDialogRequestAuthority;

/// In-dialog REFER builder (RFC 3515, call transfer). Reachable via
/// [`UnifiedCoordinator::refer`](crate::api::unified::UnifiedCoordinator::refer).
pub struct ReferBuilder {
    coord: Arc<UnifiedCoordinator>,
    session_id: CallId,
    refer_to: String,
    replaces: Option<String>,
    referred_by: Option<String>,
    target_dialog: Option<String>,
    state: BuilderHeaderState,
    authority: InDialogRequestAuthority,
}

impl ReferBuilder {
    pub(crate) fn new(
        coord: Arc<UnifiedCoordinator>,
        session_id: CallId,
        refer_to: impl Into<String>,
    ) -> Self {
        Self {
            coord,
            session_id,
            refer_to: refer_to.into(),
            replaces: None,
            referred_by: None,
            target_dialog: None,
            state: BuilderHeaderState::default(),
            authority: InDialogRequestAuthority::CaptureCurrent,
        }
    }

    pub(crate) fn new_captured(
        coord: Arc<UnifiedCoordinator>,
        session_id: CallId,
        refer_to: impl Into<String>,
        lifecycle_handle: Option<SessionRegistryHandle>,
    ) -> Self {
        let mut builder = Self::new(coord, session_id, refer_to);
        builder.authority = InDialogRequestAuthority::captured(lifecycle_handle);
        builder
    }

    /// RFC 3891 `Replaces` (attended transfer).
    pub fn with_replaces(mut self, replaces: impl Into<String>) -> Self {
        self.replaces = Some(replaces.into());
        self
    }

    /// RFC 3892 `Referred-By`.
    pub fn with_referred_by(mut self, uri: impl Into<String>) -> Self {
        self.referred_by = Some(uri.into());
        self
    }

    /// RFC 4538 `Target-Dialog`. Builds the
    /// `<call-id>;local-tag=<from-tag>;remote-tag=<to-tag>` value from
    /// the supplied request's dialog identifiers. Local- and remote-tag
    /// parameters are emitted only when present on the underlying
    /// request — RFC 4538 §3 permits omission for early dialogs.
    pub fn with_target_dialog(mut self, request: &crate::api::incoming::IncomingRequest) -> Self {
        if let Some(req) = request.raw_request() {
            if let Some(cid) = req.call_id() {
                let mut value = cid.to_string();
                if let Some(tag) = req.from().and_then(|f| f.tag()) {
                    value.push_str(";local-tag=");
                    value.push_str(tag);
                }
                if let Some(tag) = req.to().and_then(|t| t.tag()) {
                    value.push_str(";remote-tag=");
                    value.push_str(tag);
                }
                self.target_dialog = Some(value);
            }
        }
        self
    }

    /// Send the REFER through the dialog's state machine.
    pub async fn send(mut self) -> Result<()> {
        let extra_headers = take_staged(&mut self.state);
        let opts = Arc::new(rvoip_sip_dialog::api::unified::ReferRequestOptions {
            refer_to: self.refer_to,
            replaces: self.replaces,
            referred_by: self.referred_by,
            target_dialog: self.target_dialog,
            extra_headers,
        });
        let event = crate::state_table::EventType::SendOutboundRefer;
        let slot = crate::state_machine::executor::PendingOptionsSlot::Refer(opts);
        match self.authority.exact_handle(&self.session_id)? {
            Some(handle) => {
                self.coord
                    .dispatch_outbound_with_options_exact(&handle, event, slot)
                    .await?
            }
            None => {
                self.coord
                    .dispatch_outbound_with_options(&self.session_id, event, slot)
                    .await?
            }
        };
        Ok(())
    }
}

impl SipRequestOptions for ReferBuilder {
    fn method(&self) -> Method {
        Method::Refer
    }
    fn header_state_mut(&mut self) -> &mut BuilderHeaderState {
        &mut self.state
    }
    fn header_state(&self) -> &BuilderHeaderState {
        &self.state
    }
}
