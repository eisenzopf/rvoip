//! `MessageBuilder` — SIP_API_DESIGN_2 §3.3 (RFC 3428 out-of-dialog MESSAGE).

use std::sync::Arc;

use bytes::Bytes;
use rvoip_sip_core::types::Method;

use crate::api::headers::{take_staged, BuilderHeaderState, SipRequestOptions};
use crate::api::unified::UnifiedCoordinator;
use crate::errors::Result;
use crate::types::Credentials;

pub struct MessageBuilder {
    coord: Arc<UnifiedCoordinator>,
    target: String,
    from_uri: Option<String>,
    content_type: String,
    body: Option<Bytes>,
    credentials: Option<Credentials>,
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
            state: BuilderHeaderState::default(),
        }
    }

    pub fn with_body(mut self, body: impl Into<Bytes>) -> Self {
        self.body = Some(body.into());
        self
    }
    pub fn with_content_type(mut self, ct: impl Into<String>) -> Self {
        self.content_type = ct.into();
        self
    }
    pub fn with_credentials(mut self, c: Credentials) -> Self {
        self.credentials = Some(c);
        self
    }
    pub fn with_from_uri(mut self, s: impl Into<String>) -> Self {
        self.from_uri = Some(s.into());
        self
    }

    pub async fn send(mut self) -> Result<()> {
        let from_uri = self
            .from_uri
            .clone()
            .unwrap_or_else(|| self.coord.config_local_uri());
        let body = self.body.unwrap_or_default();
        let authorization = self
            .credentials
            .as_ref()
            .map(|c| format!("Digest username=\"{}\"", c.username));
        let extra_headers = take_staged(&mut self.state);
        let opts = rvoip_sip_dialog::api::unified::MessageRequestOptions {
            from_uri,
            to_uri: self.target,
            content_type: self.content_type,
            body,
            authorization,
            extra_headers,
        };
        self.coord
            .dialog_adapter()
            .send_message_oob_with_options(opts)
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
