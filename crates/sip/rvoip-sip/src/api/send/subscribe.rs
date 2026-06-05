//! `SubscribeBuilder` / `SubscribeRefreshBuilder` — SIP_API_DESIGN_2 §3.3.

use std::sync::Arc;

use rvoip_sip_core::types::Method;

use crate::api::handle::CallId;
use crate::api::headers::{take_staged, BuilderHeaderState, SipRequestOptions};
use crate::api::unified::UnifiedCoordinator;
use crate::auth::SipClientAuth;
use crate::errors::Result;
use crate::types::Credentials;

/// Opaque handle returned by `SubscribeBuilder::send()`. Carries the
/// session id so refresh / unsubscribe can route through the same
/// dialog without the caller threading state.
pub struct SubscriptionHandle {
    /// Internal correlation id (mirrors the SIP Call-ID).
    pub id: String,
    /// Session id under which the SUBSCRIBE dialog is registered. Set
    /// when the initial SUBSCRIBE establishes a dialog; refresh /
    /// unsubscribe sends route through this session.
    pub(crate) session_id: Option<CallId>,
    /// Reference to the coordinator so the handle can dispatch refresh
    /// builders without requiring the caller to re-thread it.
    pub(crate) coord: Option<Arc<UnifiedCoordinator>>,
    /// Cached event package; refresh reuses the same package.
    pub(crate) event_package: String,
    /// Original request target used for direct refresh/unsubscribe sends.
    pub(crate) target: String,
    /// Original From URI; direct refreshes reuse it because there is no
    /// session-state record for out-of-dialog subscriptions.
    pub(crate) from_uri: String,
    /// Original Contact URI if supplied by the caller.
    pub(crate) contact_uri: Option<String>,
    /// Original Accept header if supplied by the caller.
    pub(crate) accept: Option<String>,
}

impl SubscriptionHandle {
    /// Begin a refresh of this subscription.
    pub fn refresh(self) -> SubscribeRefreshBuilder {
        SubscribeRefreshBuilder::new(self)
    }
}

/// Outbound out-of-dialog SUBSCRIBE builder (RFC 6665). Reachable via
/// [`UnifiedCoordinator::subscribe`](crate::api::unified::UnifiedCoordinator::subscribe).
pub struct SubscribeBuilder {
    coord: Arc<UnifiedCoordinator>,
    target: String,
    event_package: String,
    expires: u32,
    from_uri: Option<String>,
    contact_uri: Option<String>,
    accept: Option<String>,
    credentials: Option<Credentials>,
    auth: Option<SipClientAuth>,
    state: BuilderHeaderState,
}

impl SubscribeBuilder {
    pub(crate) fn new(
        coord: Arc<UnifiedCoordinator>,
        target: impl Into<String>,
        event_package: impl Into<String>,
    ) -> Self {
        Self {
            coord,
            target: target.into(),
            event_package: event_package.into(),
            expires: 3600,
            from_uri: None,
            contact_uri: None,
            accept: None,
            credentials: None,
            auth: None,
            state: BuilderHeaderState::default(),
        }
    }

    /// Override the `From:` URI (defaults to `Config.local_uri`).
    pub fn with_from_uri(mut self, s: impl Into<String>) -> Self {
        self.from_uri = Some(s.into());
        self
    }
    /// Override the `Contact:` URI.
    pub fn with_contact_uri(mut self, s: impl Into<String>) -> Self {
        self.contact_uri = Some(s.into());
        self
    }
    /// Set the subscription lifetime via the `Expires:` header (seconds).
    pub fn with_expires(mut self, secs: u32) -> Self {
        self.expires = secs;
        self
    }
    /// Set the `Accept:` header advertising acceptable notification body types.
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

    /// Convenience entry for RFC 4235 dialog-package subscriptions.
    /// Equivalent to `self.send().await?` followed by wrapping the
    /// returned `SubscriptionHandle` in a `DialogSubscriptionHandle`
    /// that exposes typed dialog-info accessors. Panics at compile
    /// time if `event_package` was set to anything other than `"dialog"`?
    /// No — this is a runtime convenience; the builder takes whatever
    /// event package the caller passed, so misuse silently produces a
    /// `DialogSubscriptionHandle` whose `wait_for_dialog` will never
    /// fire. Callers should ensure the event package is `"dialog"`.
    pub async fn send_dialog_events(
        self,
    ) -> Result<crate::api::dialog_subscription::DialogSubscriptionHandle> {
        let target = self.target.clone();
        let handle = self.send().await?;
        crate::api::dialog_subscription::DialogSubscriptionHandle::from_subscription(handle, target)
    }

    /// Send the SUBSCRIBE, returning a [`SubscriptionHandle`] for
    /// refresh / unsubscribe.
    pub async fn send(mut self) -> Result<SubscriptionHandle> {
        let from_uri = self
            .from_uri
            .clone()
            .unwrap_or_else(|| self.coord.config_local_uri());
        let auth = self.auth.or_else(|| self.credentials.map(Into::into));
        let extra_headers = take_staged(&mut self.state);
        let opts = rvoip_sip_dialog::api::unified::SubscribeRequestOptions {
            event: self.event_package.clone(),
            expires: self.expires,
            accept: self.accept.clone(),
            from_uri: Some(from_uri.clone()),
            contact_uri: self.contact_uri.clone(),
            authorization: None,
            cseq: None,
            call_id: None,
            from_tag: None,
            refresh: false,
            extra_headers,
        };
        // SUBSCRIBE is intentionally direct-wired. Earlier code sent the
        // initial request directly but attempted refresh through the session
        // state machine, even though no session record was created for the
        // returned handle. Keep the whole public SUBSCRIBE builder path on
        // the direct adapter route so it cannot dead-end on SessionNotFound.
        let response = self
            .coord
            .send_subscribe_oob_with_optional_auth(&self.target, opts, auth)
            .await?;
        let id = response
            .call_id()
            .map(|c| c.to_string())
            .unwrap_or_else(|| format!("subscription-{}", uuid::Uuid::new_v4()));
        let session_id = crate::state_table::types::SessionId(id.clone());
        Ok(SubscriptionHandle {
            id,
            session_id: Some(session_id),
            coord: Some(self.coord.clone()),
            event_package: self.event_package,
            target: self.target,
            from_uri,
            contact_uri: self.contact_uri,
            accept: self.accept,
        })
    }
}

impl SipRequestOptions for SubscribeBuilder {
    fn method(&self) -> Method {
        Method::Subscribe
    }
    fn header_state_mut(&mut self) -> &mut BuilderHeaderState {
        &mut self.state
    }
    fn header_state(&self) -> &BuilderHeaderState {
        &self.state
    }
}

/// Builder that refreshes an existing subscription within the same
/// dialog, reusing the original event package, target and `From:`.
pub struct SubscribeRefreshBuilder {
    handle: SubscriptionHandle,
    expires: Option<u32>,
    credentials: Option<Credentials>,
    auth: Option<SipClientAuth>,
    state: BuilderHeaderState,
}

impl SubscribeRefreshBuilder {
    fn new(handle: SubscriptionHandle) -> Self {
        Self {
            handle,
            expires: None,
            credentials: None,
            auth: None,
            state: BuilderHeaderState::default(),
        }
    }

    /// Override the refresh `Expires:` (seconds); defaults to the
    /// original subscription's interval.
    pub fn with_expires(mut self, secs: u32) -> Self {
        self.expires = Some(secs);
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

    /// Send the refresh SUBSCRIBE through the subscription's dialog.
    pub async fn send(mut self) -> Result<()> {
        let coord = self.handle.coord.clone().ok_or_else(|| {
            crate::errors::SessionError::InternalError(
                "SubscribeRefreshBuilder.send(): subscription handle is detached \
                 (no coordinator) — only handles returned by SubscribeBuilder.send() \
                 can be refreshed"
                    .to_string(),
            )
        })?;
        let auth = self.auth.or_else(|| self.credentials.map(Into::into));
        let extra_headers = take_staged(&mut self.state);
        let opts = rvoip_sip_dialog::api::unified::SubscribeRequestOptions {
            event: self.handle.event_package.clone(),
            // Spec §7.1: refresh reuses the original interval unless the
            // caller overrides; default to 3600 if neither is provided.
            expires: self.expires.unwrap_or(3600),
            accept: self.handle.accept.clone(),
            from_uri: Some(self.handle.from_uri.clone()),
            contact_uri: self.handle.contact_uri.clone(),
            authorization: None,
            cseq: None,
            call_id: None,
            from_tag: None,
            refresh: true,
            extra_headers,
        };
        coord
            .send_subscribe_oob_with_optional_auth(&self.handle.target, opts, auth)
            .await?;
        Ok(())
    }
}

impl SipRequestOptions for SubscribeRefreshBuilder {
    fn method(&self) -> Method {
        Method::Subscribe
    }
    fn header_state_mut(&mut self) -> &mut BuilderHeaderState {
        &mut self.state
    }
    fn header_state(&self) -> &BuilderHeaderState {
        &self.state
    }
}
