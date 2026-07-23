//! `ByeBuilder` — SIP_API_DESIGN_2 §3.3.

use std::sync::Arc;

use rvoip_sip_core::types::Method;

use crate::api::handle::CallId;
use crate::api::headers::{take_staged, BuilderHeaderState, SipRequestOptions};
use crate::api::unified::UnifiedCoordinator;
use crate::errors::Result;
use crate::session_registry::SessionRegistryHandle;

use super::InDialogRequestAuthority;

/// Outbound BYE builder. Reachable via
/// [`UnifiedCoordinator::bye`](crate::api::unified::UnifiedCoordinator::bye).
pub struct ByeBuilder {
    coord: Arc<UnifiedCoordinator>,
    session_id: CallId,
    reason: Option<String>,
    state: BuilderHeaderState,
    authority: InDialogRequestAuthority,
}

impl ByeBuilder {
    pub(crate) fn new(coord: Arc<UnifiedCoordinator>, session_id: CallId) -> Self {
        Self {
            coord,
            session_id,
            reason: None,
            state: BuilderHeaderState::default(),
            authority: InDialogRequestAuthority::CaptureCurrent,
        }
    }

    pub(crate) fn new_captured(
        coord: Arc<UnifiedCoordinator>,
        session_id: CallId,
        lifecycle_handle: Option<SessionRegistryHandle>,
    ) -> Self {
        let mut builder = Self::new(coord, session_id);
        builder.authority = InDialogRequestAuthority::captured(lifecycle_handle);
        builder
    }

    /// Attach an RFC 3326 `Reason:` header to the BYE.
    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }

    /// Attach a structured RFC 3326 `Reason:` header (preserving
    /// `protocol`/`cause`/`text` exactly — e.g. `Q.850 ;cause=16 ;text="…"`).
    ///
    /// Use this when the caller needs a non-SIP protocol token or a
    /// numeric `cause` other than 200; [`with_reason`](Self::with_reason)
    /// is a shorthand that always renders as `SIP ;cause=200 ;text="…"`.
    pub fn with_sip_reason(mut self, reason: crate::api::handle::SipReason) -> Self {
        let typed = rvoip_sip_core::types::headers::TypedHeader::Reason(
            rvoip_sip_core::types::reason::Reason::new(reason.protocol, reason.cause, reason.text),
        );
        self.state.headers.push(typed);
        self
    }

    /// Send the BYE through rvoip-sip-dialog's
    /// `send_bye_with_options` so staged application headers ride to
    /// the wire after stack-managed headers are stamped.
    pub async fn send(mut self) -> Result<()> {
        let extra_headers = take_staged(&mut self.state);
        let opts = Arc::new(rvoip_sip_dialog::api::unified::ByeRequestOptions {
            reason: self.reason,
            extra_headers,
        });
        let slot = crate::state_machine::executor::PendingOptionsSlot::Bye(opts);
        match self.authority.exact_handle(&self.session_id)? {
            Some(handle) => {
                self.coord
                    .dispatch_confirmed_outbound_bye_exact(&handle, slot)
                    .await?;
            }
            None => {
                self.coord
                    .dispatch_confirmed_outbound_bye(&self.session_id, slot)
                    .await?;
            }
        }
        Ok(())
    }
}

impl SipRequestOptions for ByeBuilder {
    fn method(&self) -> Method {
        Method::Bye
    }
    fn header_state_mut(&mut self) -> &mut BuilderHeaderState {
        &mut self.state
    }
    fn header_state(&self) -> &BuilderHeaderState {
        &self.state
    }
}
