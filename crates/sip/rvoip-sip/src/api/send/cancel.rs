//! `CancelBuilder` — SIP_API_DESIGN_2 §3.3.

use std::sync::Arc;

use rvoip_sip_core::types::Method;

use crate::api::handle::CallId;
use crate::api::headers::{take_staged, BuilderHeaderState, SipRequestOptions};
use crate::api::unified::UnifiedCoordinator;
use crate::errors::Result;
use crate::session_registry::SessionRegistryHandle;

use super::InDialogRequestAuthority;

/// Outbound CANCEL builder (RFC 3261 §9, abort a pending INVITE).
/// Reachable via
/// [`UnifiedCoordinator::cancel`](crate::api::unified::UnifiedCoordinator::cancel).
pub struct CancelBuilder {
    coord: Arc<UnifiedCoordinator>,
    session_id: CallId,
    reason: Option<String>,
    state: BuilderHeaderState,
    authority: InDialogRequestAuthority,
}

impl CancelBuilder {
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

    /// RFC 3326 `Reason:` header attached to the CANCEL. Stamped on
    /// the wire alongside any application-staged headers.
    pub fn with_reason(mut self, reason: impl Into<String>) -> Self {
        self.reason = Some(reason.into());
        self
    }

    /// Send the CANCEL. RFC 3261 §9.1 — CANCEL targets the
    /// most-recently-sent INVITE on the session. The `Reason` (if
    /// any) and application-staged extras are appended to the
    /// generated CANCEL after the stack-managed slice per §5.2.
    pub async fn send(mut self) -> Result<()> {
        let extra_headers = take_staged(&mut self.state);
        let opts = Arc::new(rvoip_sip_dialog::api::unified::CancelRequestOptions {
            reason: self.reason,
            extra_headers,
        });
        let event = crate::state_table::EventType::SendOutboundCancel;
        let slot = crate::state_machine::executor::PendingOptionsSlot::Cancel(opts);
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

impl SipRequestOptions for CancelBuilder {
    fn method(&self) -> Method {
        Method::Cancel
    }
    fn header_state_mut(&mut self) -> &mut BuilderHeaderState {
        &mut self.state
    }
    fn header_state(&self) -> &BuilderHeaderState {
        &self.state
    }
}
