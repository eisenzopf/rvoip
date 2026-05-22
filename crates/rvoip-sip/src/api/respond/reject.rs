//! `RejectBuilder` — SIP_API_DESIGN_2 §3.4.

use std::sync::Arc;

use rvoip_sip_core::types::headers::{HeaderName, HeaderValue, TypedHeader};
use rvoip_sip_core::types::Method;

use crate::api::handle::CallId;
use crate::api::headers::{take_staged, BuilderHeaderState, SipRequestOptions};
use crate::api::unified::UnifiedCoordinator;
use crate::errors::Result;

pub struct RejectBuilder {
    coord: Arc<UnifiedCoordinator>,
    call_id: CallId,
    status: u16,
    reason: Option<String>,
    retry_after: Option<u32>,
    /// Pre-rendered `Warning:` header values (RFC 3261 §20.43). Each
    /// entry is the body of a separate `Warning:` line; multiple
    /// warnings on one response are RFC-compliant.
    warnings: Vec<String>,
    state: BuilderHeaderState,
}

impl RejectBuilder {
    pub(crate) fn new(coord: Arc<UnifiedCoordinator>, call_id: CallId) -> Self {
        Self {
            coord,
            call_id,
            status: 486,
            reason: None,
            retry_after: None,
            warnings: Vec::new(),
            state: BuilderHeaderState::default(),
        }
    }

    pub fn with_status(mut self, code: u16) -> Self {
        self.status = code;
        self
    }
    pub fn with_reason(mut self, r: impl Into<String>) -> Self {
        self.reason = Some(r.into());
        self
    }
    pub fn with_retry_after(mut self, secs: u32) -> Self {
        self.retry_after = Some(secs);
        self
    }

    /// Attach an RFC 3261 §20.43 `Warning:` header. The wire format
    /// is `<3-digit code> <agent> "<text>"`. Multiple warnings may be
    /// stacked on a single response.
    pub fn with_warning(mut self, code: u16, agent: &str, text: &str) -> Self {
        // Escape inner quotes so the warn-text token stays well-formed.
        let escaped = text.replace('"', r#"\""#);
        self.warnings.push(format!("{code} {agent} \"{escaped}\""));
        self
    }

    pub async fn send(mut self) -> Result<()> {
        let reason = self
            .reason
            .clone()
            .unwrap_or_else(|| default_reason_for(self.status).to_string());
        let mut extras = take_staged(&mut self.state);

        // Stamp Retry-After / Warning into the extras list. These are
        // application-controlled per the HeaderPolicy matrix (§5.1) so
        // they ride the same extras channel as any other custom field.
        if let Some(secs) = self.retry_after {
            extras.push(TypedHeader::Other(
                HeaderName::RetryAfter,
                HeaderValue::Raw(secs.to_string().into_bytes()),
            ));
        }
        for w in &self.warnings {
            extras.push(TypedHeader::Other(
                HeaderName::Warning,
                HeaderValue::Raw(w.clone().into_bytes()),
            ));
        }

        if !extras.is_empty() {
            // SIP_API_DESIGN_2 §3.4 — stash extras on the session so
            // `Action::SendRejectResponse` picks them up on its single
            // wire dispatch. We previously sent the response twice
            // (once via `send_response_with_options`, once via
            // `reject_call → SendRejectResponse`); the second response
            // overwrote the first on the wire and dropped the staged
            // extras. The stash-then-dispatch pattern guarantees one
            // wire response with the right extras.
            let session = self.coord.session_state(&self.call_id).await.map_err(|_| {
                crate::errors::SessionError::SessionNotFound(self.call_id.to_string())
            })?;
            let mut session = session;
            session.reject_response_extras = Some(extras);
            self.coord.update_session_state(session).await?;
        }

        self.coord
            .helpers
            .reject_call(&self.call_id, self.status, &reason)
            .await
    }
}

impl SipRequestOptions for RejectBuilder {
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

fn default_reason_for(status: u16) -> &'static str {
    match status {
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        408 => "Request Timeout",
        480 => "Temporarily Unavailable",
        486 => "Busy Here",
        487 => "Request Terminated",
        488 => "Not Acceptable Here",
        500 => "Server Internal Error",
        503 => "Service Unavailable",
        603 => "Decline",
        _ => "Rejected",
    }
}
