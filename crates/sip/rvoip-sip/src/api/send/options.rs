//! `OptionsBuilder` — SIP_API_DESIGN_2 §3.3 (out-of-dialog OPTIONS).

use std::sync::Arc;
use std::time::Duration;

use rvoip_sip_core::types::Method;

use crate::api::headers::{take_staged, BuilderHeaderState, SipRequestOptions};
use crate::api::incoming::IncomingResponse;
use crate::api::unified::UnifiedCoordinator;
use crate::auth::SipClientAuth;
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
    auth: Option<SipClientAuth>,
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
            auth: None,
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
    /// Attach Digest credentials for UAC 401/407 retry.
    pub fn with_credentials(mut self, c: Credentials) -> Self {
        self.credentials = Some(c);
        self
    }
    /// Attach general UAC SIP auth for 401/407 retry.
    ///
    /// Use [`SipClientAuth::any`] when the peer may offer multiple schemes and
    /// the UAC should negotiate among Digest, Bearer, Basic, and AKA options.
    pub fn with_auth(mut self, auth: SipClientAuth) -> Self {
        self.auth = Some(auth);
        self
    }
    /// Attach a Bearer token for UAC 401/407 retry.
    pub fn with_bearer_token(mut self, token: impl Into<String>) -> Self {
        self.auth = Some(SipClientAuth::bearer_token(token));
        self
    }
    /// Attach Basic credentials for UAC 401/407 retry.
    ///
    /// Basic is cleartext-disabled by default. Use
    /// `with_auth(SipClientAuth::basic(...).allow_basic_over_cleartext(true))`
    /// only for explicit legacy cleartext interop.
    pub fn with_basic_credentials(
        mut self,
        username: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        self.auth = Some(SipClientAuth::basic(username, password));
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
        let auth = self.auth.or_else(|| self.credentials.map(Into::into));
        let response = self
            .coord
            .send_options_oob_with_optional_auth(opts, auth)
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
