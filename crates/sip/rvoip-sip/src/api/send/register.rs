//! `RegisterBuilder` / `RegisterRefreshBuilder` — SIP_API_DESIGN_2 §3.3.

use std::sync::Arc;

use rvoip_sip_core::types::Method;

use crate::api::headers::{take_staged, BuilderHeaderState, SipRequestOptions};
use crate::api::unified::{RegistrationHandle, UnifiedCoordinator};
use crate::errors::Result;
use crate::session_registry::SessionRegistryHandle;

/// Outbound REGISTER builder (RFC 3261 §10). Reachable via
/// [`UnifiedCoordinator::register`](crate::api::unified::UnifiedCoordinator::register).
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

    /// Set the registration lifetime via the `Expires:` header (seconds).
    pub fn with_expires(mut self, secs: u32) -> Self {
        self.expires = secs;
        self
    }
    /// Override the `From:` URI (defaults to `Config.local_uri`).
    pub fn with_from_uri(mut self, s: impl Into<String>) -> Self {
        self.from_uri = Some(s.into());
        self
    }
    /// Override the `Contact:` URI being registered.
    pub fn with_contact_uri(mut self, s: impl Into<String>) -> Self {
        self.contact_uri = Some(s.into());
        self
    }
    /// Route the REGISTER through an outbound proxy `Route:`.
    pub fn with_outbound_proxy(mut self, s: impl Into<String>) -> Self {
        self.outbound_proxy = Some(s.into());
        self
    }
    /// Suppress the outbound proxy `Route:` even when configured.
    pub fn without_outbound_proxy(mut self) -> Self {
        self.suppress_outbound_proxy = true;
        self
    }
    /// Add an RFC 3327 `Path:` header.
    pub fn with_path(mut self, uri: impl Into<String>) -> Self {
        self.path = Some(uri.into());
        self
    }
    /// Set the `Contact:` `q` value (RFC 3261 preference weighting).
    pub fn with_q_value(mut self, q: f32) -> Self {
        self.q_value = Some(q);
        self
    }
    /// Set the RFC 5626 `+sip.instance` Contact parameter (instance URN).
    pub fn with_sip_instance(mut self, urn: impl Into<String>) -> Self {
        self.sip_instance = Some(urn.into());
        self
    }
    /// Set the RFC 5626 `reg-id` Contact parameter.
    pub fn with_reg_id(mut self, id: u32) -> Self {
        self.reg_id = Some(id);
        self
    }
    /// Pre-computed `Authorization:` header value, bypassing 401-driven
    /// digest computation.
    pub fn with_precomputed_authorization(mut self, s: impl Into<String>) -> Self {
        self.precomputed_authorization = Some(s.into());
        self
    }

    /// Send the REGISTER, returning a [`RegistrationHandle`] for refresh.
    pub async fn send(mut self) -> Result<RegistrationHandle> {
        let from_uri = self
            .from_uri
            .clone()
            .unwrap_or_else(|| self.coord.config_local_uri());
        // The Contact must be the reachable transport address so the registrar
        // routes inbound calls back to us — default to the bound/advertised
        // address (or an explicit `Config.contact_uri`), NOT the port-less AOR
        // that `from_uri` carries. Override with `with_contact_uri()`.
        let contact_uri = self
            .contact_uri
            .clone()
            .unwrap_or_else(|| self.coord.config_contact_uri(&self.user));
        let extra_headers = take_staged(&mut self.state);

        // SIP_API_DESIGN_2 §10 #19 — application-staged extras (raw
        // `P-Asserted-Identity`, custom `X-*`, RFC 3327 `Path`, …) ride
        // through rvoip-sip-dialog's `extra_headers` channel. The empty-extras
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

/// Builder that refreshes an existing registration, reusing the
/// original Call-ID / AoR / contact while incrementing CSeq.
pub struct RegisterRefreshBuilder {
    coord: Arc<UnifiedCoordinator>,
    handle: RegistrationHandle,
    lifecycle_handle: Option<SessionRegistryHandle>,
    expires: Option<u32>,
    state: BuilderHeaderState,
}

impl RegisterRefreshBuilder {
    pub(crate) fn new(
        coord: Arc<UnifiedCoordinator>,
        handle: RegistrationHandle,
        lifecycle_handle: Option<SessionRegistryHandle>,
    ) -> Self {
        Self {
            coord,
            handle,
            lifecycle_handle,
            expires: None,
            state: BuilderHeaderState::default(),
        }
    }

    /// Override the refresh `Expires:` (seconds); defaults to the
    /// original registration's interval.
    pub fn with_expires(mut self, secs: u32) -> Self {
        self.expires = Some(secs);
        self
    }

    /// Refresh the registration.
    ///
    /// The builder retains the exact registration lifetime captured when it
    /// was constructed. Its Call-ID, addressing metadata, and `CSeq + 1` are
    /// derived only after that generation's state-machine lane is acquired;
    /// the immutable options snapshot is then staged and dispatched without
    /// releasing the lane. This preserves RFC 3261 §10.2.4 identity and makes
    /// concurrent manual/automatic refreshes one ordered CSeq history.
    pub async fn send(mut self) -> Result<()> {
        let lifecycle_handle = self.lifecycle_handle.take().ok_or_else(|| {
            crate::errors::SessionError::SessionNotFound(format!(
                "Session {} has no exact registration refresh authority",
                self.handle.session_id
            ))
        })?;
        if lifecycle_handle.session_id() != &self.handle.session_id {
            return Err(crate::errors::SessionError::InvalidTransition(
                "captured registration refresh authority does not match its session".to_string(),
            ));
        }
        let extra_headers = take_staged(&mut self.state);
        self.coord
            .dispatch_registration_refresh_exact(&lifecycle_handle, self.expires, extra_headers)
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

#[cfg(test)]
mod tests {
    use super::*;
    use crate::api::unified::Config;
    use crate::state_table::Role;
    use crate::types::SessionId;
    use std::time::Duration;

    #[tokio::test]
    async fn refresh_builder_cannot_cross_a_reused_session_generation() {
        let coordinator = UnifiedCoordinator::new(Config::local("refresh-builder-generation", 0))
            .await
            .expect("create refresh builder coordinator");
        let store = Arc::clone(&coordinator.helpers.state_machine.store);
        let session_id = SessionId("refresh-builder-reused-id".to_string());
        let created_a = store
            .create_session(session_id.clone(), Role::UAC, false)
            .await
            .expect("create registration generation A");
        let handle_a = created_a
            .lifecycle_handle
            .clone()
            .expect("capture generation A lifecycle");
        let public_handle = RegistrationHandle {
            session_id: session_id.clone(),
        };
        let builder = coordinator.refresh(&public_handle);

        store
            .remove_session_exact(&handle_a)
            .await
            .expect("retire registration generation A");
        assert!(store.authority().elapse_reuse_horizon_for_test(&session_id));
        let created_b = store
            .create_session(session_id.clone(), Role::UAC, false)
            .await
            .expect("create registration generation B");
        let handle_b = created_b
            .lifecycle_handle
            .clone()
            .expect("capture generation B lifecycle");
        store
            .update_session_exact_with(&handle_b, None, |session| {
                session.registration_call_id = Some("generation-b-call-id".to_string());
                session.registration_cseq = 77;
            })
            .expect("mark generation B registration identity");
        let before = store
            .get_session_snapshot_exact(&handle_b)
            .expect("read generation B before stale builder");

        assert!(builder.send().await.is_err());

        let after = store
            .get_session_snapshot_exact(&handle_b)
            .expect("read generation B after stale builder");
        assert_eq!(after.revision(), before.revision());
        assert_eq!(after.registration_call_id, before.registration_call_id);
        assert_eq!(after.registration_cseq, 77);

        coordinator
            .shutdown_gracefully(Some(Duration::from_secs(1)))
            .await
            .expect("shutdown refresh builder coordinator");
    }
}
