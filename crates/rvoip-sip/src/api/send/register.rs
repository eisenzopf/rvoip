//! `RegisterBuilder` / `RegisterRefreshBuilder` — SIP_API_DESIGN_2 §3.3.

use std::sync::Arc;

use rvoip_sip_core::types::Method;

use crate::api::headers::{take_staged, BuilderHeaderState, SipRequestOptions};
use crate::api::unified::{RegistrationHandle, UnifiedCoordinator};
use crate::errors::Result;

pub struct RegisterBuilder {
    coord: Arc<UnifiedCoordinator>,
    registrar: String,
    user: String,
    password: String,
    expires: u32,
    from_uri: Option<String>,
    contact_uri: Option<String>,
    outbound_proxy: Option<String>,
    suppress_outbound_proxy: bool,
    path: Option<String>,
    q_value: Option<f32>,
    sip_instance: Option<String>,
    reg_id: Option<u32>,
    precomputed_authorization: Option<String>,
    state: BuilderHeaderState,
}

impl RegisterBuilder {
    pub(crate) fn new(
        coord: Arc<UnifiedCoordinator>,
        registrar: impl Into<String>,
        user: impl Into<String>,
        password: impl Into<String>,
    ) -> Self {
        Self {
            coord,
            registrar: registrar.into(),
            user: user.into(),
            password: password.into(),
            expires: 3600,
            from_uri: None,
            contact_uri: None,
            outbound_proxy: None,
            suppress_outbound_proxy: false,
            path: None,
            q_value: None,
            sip_instance: None,
            reg_id: None,
            precomputed_authorization: None,
            state: BuilderHeaderState::default(),
        }
    }

    pub fn with_expires(mut self, secs: u32) -> Self {
        self.expires = secs;
        self
    }
    pub fn with_from_uri(mut self, s: impl Into<String>) -> Self {
        self.from_uri = Some(s.into());
        self
    }
    pub fn with_contact_uri(mut self, s: impl Into<String>) -> Self {
        self.contact_uri = Some(s.into());
        self
    }
    pub fn with_outbound_proxy(mut self, s: impl Into<String>) -> Self {
        self.outbound_proxy = Some(s.into());
        self
    }
    pub fn without_outbound_proxy(mut self) -> Self {
        self.suppress_outbound_proxy = true;
        self
    }
    pub fn with_path(mut self, uri: impl Into<String>) -> Self {
        self.path = Some(uri.into());
        self
    }
    pub fn with_q_value(mut self, q: f32) -> Self {
        self.q_value = Some(q);
        self
    }
    pub fn with_sip_instance(mut self, urn: impl Into<String>) -> Self {
        self.sip_instance = Some(urn.into());
        self
    }
    pub fn with_reg_id(mut self, id: u32) -> Self {
        self.reg_id = Some(id);
        self
    }
    pub fn with_precomputed_authorization(mut self, s: impl Into<String>) -> Self {
        self.precomputed_authorization = Some(s.into());
        self
    }

    pub async fn send(mut self) -> Result<RegistrationHandle> {
        let from_uri = self
            .from_uri
            .clone()
            .unwrap_or_else(|| self.coord.config_local_uri());
        let contact_uri = self.contact_uri.clone().unwrap_or_else(|| from_uri.clone());
        let extra_headers = take_staged(&mut self.state);

        // SIP_API_DESIGN_2 §10 #19 — application-staged extras (raw
        // `P-Asserted-Identity`, custom `X-*`, RFC 3327 `Path`, …) ride
        // through dialog-core's `extra_headers` channel. The empty-extras
        // case (auth-retry / 423-retry / plain register) takes the same
        // path; the slice is just empty.
        self.coord
            .register_with_extras(
                &self.registrar,
                &from_uri,
                &contact_uri,
                &self.user,
                &self.password,
                self.expires,
                extra_headers,
            )
            .await
    }
}

impl SipRequestOptions for RegisterBuilder {
    fn method(&self) -> Method {
        Method::Register
    }
    fn header_state_mut(&mut self) -> &mut BuilderHeaderState {
        &mut self.state
    }
    fn header_state(&self) -> &BuilderHeaderState {
        &self.state
    }
}

pub struct RegisterRefreshBuilder {
    coord: Arc<UnifiedCoordinator>,
    handle: RegistrationHandle,
    expires: Option<u32>,
    state: BuilderHeaderState,
}

impl RegisterRefreshBuilder {
    pub(crate) fn new(coord: Arc<UnifiedCoordinator>, handle: RegistrationHandle) -> Self {
        Self {
            coord,
            handle,
            expires: None,
            state: BuilderHeaderState::default(),
        }
    }

    pub fn with_expires(mut self, secs: u32) -> Self {
        self.expires = Some(secs);
        self
    }

    /// Refresh the registration.
    ///
    /// Stages a `RegisterRequestOptions { refresh: true, expires,
    /// extra_headers, ... }` snapshot on the registration's session
    /// and dispatches `EventType::SendOutboundRegister`. The state
    /// table routes to `Action::SendREGISTERWithOptions` which drains
    /// the stash via the dialog-adapter mirror. Call-ID is preserved
    /// (RFC 3261 §10.2.4), CSeq incremented, and the requested
    /// `Expires` carried verbatim.
    pub async fn send(mut self) -> Result<()> {
        // Read the existing registration's metadata off the session so
        // the refresh REGISTER reuses the AoR / contact / registrar of
        // the original registration.
        let session = self
            .coord
            .session_state(&self.handle.session_id)
            .await
            .map_err(|_| {
                crate::errors::SessionError::SessionNotFound(self.handle.session_id.to_string())
            })?;

        let registrar_uri = session.registrar_uri.clone().unwrap_or_default();
        let contact_uri = session.registration_contact.clone().unwrap_or_default();
        let aor_uri = session.local_uri.clone().unwrap_or_else(|| contact_uri.clone());
        let expires = self
            .expires
            .or(session.registration_expires)
            .unwrap_or(3600);
        let call_id = session.registration_call_id.clone();
        let cseq = if session.registration_cseq > 0 {
            Some(session.registration_cseq + 1)
        } else {
            None
        };
        let extra_headers = take_staged(&mut self.state);
        let opts = Arc::new(rvoip_sip_dialog::api::unified::RegisterRequestOptions {
            registrar_uri,
            aor_uri,
            contact_uri,
            expires,
            authorization: None,
            call_id,
            cseq,
            outbound_contact: None,
            outbound_proxy_uri: None,
            extra_headers,
            refresh: true,
        });
        self.coord
            .stage_outbound_options(
                &self.handle.session_id,
                crate::state_machine::executor::PendingOptionsSlot::Register(opts),
            )
            .await?;
        self.coord
            .dispatch_outbound(
                &self.handle.session_id,
                crate::state_table::EventType::SendOutboundRegister,
            )
            .await?;
        Ok(())
    }
}

impl SipRequestOptions for RegisterRefreshBuilder {
    fn method(&self) -> Method {
        Method::Register
    }
    fn header_state_mut(&mut self) -> &mut BuilderHeaderState {
        &mut self.state
    }
    fn header_state(&self) -> &BuilderHeaderState {
        &self.state
    }
}
