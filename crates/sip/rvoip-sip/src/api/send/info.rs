//! `InfoBuilder` — SIP_API_DESIGN_2 §3.3.

use std::sync::Arc;

use bytes::Bytes;
use rvoip_sip_core::types::Method;

use crate::api::handle::CallId;
use crate::api::headers::{take_staged, BuilderHeaderState, SipRequestOptions};
use crate::api::unified::UnifiedCoordinator;
use crate::errors::Result;
use crate::session_registry::SessionRegistryHandle;

use super::InDialogRequestAuthority;

/// In-dialog INFO builder (RFC 6086). Reachable via
/// [`UnifiedCoordinator::info`](crate::api::unified::UnifiedCoordinator::info).
pub struct InfoBuilder {
    coord: Arc<UnifiedCoordinator>,
    session_id: CallId,
    content_type: String,
    body: Option<Bytes>,
    state: BuilderHeaderState,
    authority: InDialogRequestAuthority,
}

impl InfoBuilder {
    pub(crate) fn new(
        coord: Arc<UnifiedCoordinator>,
        session_id: CallId,
        content_type: impl Into<String>,
    ) -> Self {
        Self {
            coord,
            session_id,
            content_type: content_type.into(),
            body: None,
            state: BuilderHeaderState::default(),
            authority: InDialogRequestAuthority::CaptureCurrent,
        }
    }

    pub(crate) fn new_captured(
        coord: Arc<UnifiedCoordinator>,
        session_id: CallId,
        content_type: impl Into<String>,
        lifecycle_handle: Option<SessionRegistryHandle>,
    ) -> Self {
        let mut builder = Self::new(coord, session_id, content_type);
        builder.authority = InDialogRequestAuthority::captured(lifecycle_handle);
        builder
    }

    /// Attach the INFO request body.
    pub fn with_body(mut self, body: impl Into<Bytes>) -> Self {
        self.body = Some(body.into());
        self
    }

    /// Send the INFO through the dialog's state machine.
    pub async fn send(mut self) -> Result<()> {
        let body = self.body.unwrap_or_default();
        let extra_headers = take_staged(&mut self.state);
        let opts = Arc::new(rvoip_sip_dialog::api::unified::InfoRequestOptions {
            content_type: self.content_type,
            body,
            extra_headers,
        });
        let event = crate::state_table::EventType::SendOutboundInfo;
        let slot = crate::state_machine::executor::PendingOptionsSlot::Info(opts);
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

impl SipRequestOptions for InfoBuilder {
    fn method(&self) -> Method {
        Method::Info
    }
    fn header_state_mut(&mut self) -> &mut BuilderHeaderState {
        &mut self.state
    }
    fn header_state(&self) -> &BuilderHeaderState {
        &self.state
    }
}
