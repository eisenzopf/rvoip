//! `MessageBuilder` — SIP_API_DESIGN_2 §3.3 (RFC 3428 out-of-dialog MESSAGE).

use std::sync::Arc;

use bytes::Bytes;
use rvoip_sip_core::types::Method;

use crate::api::headers::{take_staged, BuilderHeaderState, SipRequestOptions};
use crate::api::unified::UnifiedCoordinator;
use crate::auth::SipClientAuth;
use crate::errors::Result;
use crate::types::Credentials;

/// Outbound out-of-dialog MESSAGE builder (RFC 3428). Reachable via
/// [`UnifiedCoordinator::message`](crate::api::unified::UnifiedCoordinator::message).
pub struct MessageBuilder {
    coord: Arc<UnifiedCoordinator>,
    target: String,
    from_uri: Option<String>,
    content_type: String,
    body: Option<Bytes>,
    credentials: Option<Credentials>,
    auth: Option<SipClientAuth>,
    state: BuilderHeaderState,
}

impl MessageBuilder {
    pub(crate) fn new(coord: Arc<UnifiedCoordinator>, target: impl Into<String>) -> Self {
        Self {
            coord,
            target: target.into(),
            from_uri: None,
            content_type: "text/plain".to_string(),
            body: None,
            credentials: None,
            auth: None,
            state: BuilderHeaderState::default(),
        }
    }

    /// Attach the message body.
    pub fn with_body(mut self, body: impl Into<Bytes>) -> Self {
        self.body = Some(body.into());
        self
    }
    /// Set the body's `Content-Type:` (defaults to `text/plain`).
    pub fn with_content_type(mut self, ct: impl Into<String>) -> Self {
        self.content_type = ct.into();
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
    /// Override the `From:` URI (defaults to `Config.local_uri`).
    pub fn with_from_uri(mut self, s: impl Into<String>) -> Self {
        self.from_uri = Some(s.into());
        self
    }

    /// Send the MESSAGE.
    pub async fn send(mut self) -> Result<()> {
        let from_uri = self
            .from_uri
            .clone()
            .unwrap_or_else(|| self.coord.config_local_uri());
        let body = self.body.unwrap_or_default();
        let credentials = self.credentials.clone();
        let extra_headers = take_staged(&mut self.state);
        let opts = rvoip_sip_dialog::api::unified::MessageRequestOptions {
            from_uri,
            to_uri: self.target,
            content_type: self.content_type,
            body,
            authorization: None,
            extra_headers,
        };
        self.coord
            .send_message_oob_with_optional_auth(
                opts,
                self.auth.or_else(|| credentials.map(Into::into)),
            )
            .await
            .map(|_response| ())
    }
}

impl SipRequestOptions for MessageBuilder {
    fn method(&self) -> Method {
        Method::Message
    }
    fn header_state_mut(&mut self) -> &mut BuilderHeaderState {
        &mut self.state
    }
    fn header_state(&self) -> &BuilderHeaderState {
        &self.state
    }
}
