//! `OptionsBuilder` — SIP_API_DESIGN_2 §3.3 (out-of-dialog OPTIONS).

use std::sync::Arc;
use std::time::Duration;

use rvoip_sip_core::types::Method;

use crate::api::headers::{take_staged, BuilderHeaderState, SipRequestOptions};
use crate::api::incoming::IncomingResponse;
use crate::api::unified::UnifiedCoordinator;
use crate::errors::Result;
use crate::types::Credentials;

/// Outbound out-of-dialog OPTIONS builder (RFC 3261 §11). Reachable via
/// [`UnifiedCoordinator::options`](crate::api::unified::UnifiedCoordinator::options).
pub struct OptionsBuilder {
    coord: Arc<UnifiedCoordinator>,
    target: String,
    from_uri: Option<String>,
    accept: Option<String>,
    credentials: Option<Credentials>,
    timeout: Option<Duration>,
    state: BuilderHeaderState,
}

impl OptionsBuilder {
    pub(crate) fn new(coord: Arc<UnifiedCoordinator>, target: impl Into<String>) -> Self {
        Self {
            coord,
            target: target.into(),
            from_uri: None,
            accept: None,
            credentials: None,
            timeout: None,
            state: BuilderHeaderState::default(),
        }
    }

    /// Override the `From:` URI (defaults to `Config.local_uri`).
    pub fn with_from_uri(mut self, s: impl Into<String>) -> Self {
        self.from_uri = Some(s.into());
        self
    }
    /// Set the `Accept:` header advertising acceptable response body types.
    pub fn with_accept(mut self, ct: impl Into<String>) -> Self {
        self.accept = Some(ct.into());
        self
    }
    /// Attach digest credentials for 401/407 retry.
    pub fn with_credentials(mut self, c: Credentials) -> Self {
        self.credentials = Some(c);
        self
    }
    /// Set how long to await the OPTIONS response before timing out.
    pub fn with_timeout(mut self, dur: Duration) -> Self {
        self.timeout = Some(dur);
        self
    }

    /// Send the OPTIONS and await the [`IncomingResponse`].
    pub async fn send(mut self) -> Result<IncomingResponse> {
        use crate::state_table::types::SessionId;
        let from_uri = self
            .from_uri
            .clone()
            .unwrap_or_else(|| self.coord.config_local_uri());
        let extra_headers = take_staged(&mut self.state);
        let opts = rvoip_sip_dialog::api::unified::OptionsRequestOptions {
            from_uri,
            to_uri: self.target,
            accept: self.accept,
            timeout: self.timeout,
            extra_headers,
        };
        // Credentials are staged on the builder for parity with other
        // UAC builders but rvoip-sip-dialog's options surface doesn't carry
        // an authorization slot for OPTIONS yet; the 401 retry path
        // remains application-driven.
        let _ = self.credentials;
        let response = self
            .coord
            .dialog_adapter()
            .send_options_oob_with_options(opts)
            .await?;
        let status_code: u16 = response.status_code();
        let reason_phrase = response.reason_phrase().to_string();
        let sdp = if !response.body().is_empty() {
            String::from_utf8(response.body().to_vec()).ok()
        } else {
            None
        };
        // Out-of-dialog OPTIONS produces no session, so synthesize a
        // call_id from the response's Call-ID for diagnostic
        // correlation only.
        let call_id_str = response
            .call_id()
            .map(|c| c.to_string())
            .unwrap_or_else(|| format!("options-{}", uuid::Uuid::new_v4()));
        let call_id = SessionId(call_id_str);
        Ok(IncomingResponse::with_response(
            call_id,
            status_code,
            reason_phrase,
            sdp,
            std::sync::Arc::new(response),
        ))
    }
}

impl SipRequestOptions for OptionsBuilder {
    fn method(&self) -> Method {
        Method::Options
    }
    fn header_state_mut(&mut self) -> &mut BuilderHeaderState {
        &mut self.state
    }
    fn header_state(&self) -> &BuilderHeaderState {
        &self.state
    }
}
