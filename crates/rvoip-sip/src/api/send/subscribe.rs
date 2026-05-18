//! `SubscribeBuilder` / `SubscribeRefreshBuilder` — SIP_API_DESIGN_2 §3.3.

use std::sync::Arc;

use rvoip_sip_core::types::Method;

use crate::api::handle::CallId;
use crate::api::headers::{take_staged, BuilderHeaderState, SipRequestOptions};
use crate::api::unified::UnifiedCoordinator;
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
}

impl SubscriptionHandle {
    /// Begin a refresh of this subscription.
    pub fn refresh(self) -> SubscribeRefreshBuilder {
        SubscribeRefreshBuilder::new(self)
    }
}

pub struct SubscribeBuilder {
    coord: Arc<UnifiedCoordinator>,
    target: String,
    event_package: String,
    expires: u32,
    from_uri: Option<String>,
    contact_uri: Option<String>,
    accept: Option<String>,
    credentials: Option<Credentials>,
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
            state: BuilderHeaderState::default(),
        }
    }

    pub fn with_from_uri(mut self, s: impl Into<String>) -> Self {
        self.from_uri = Some(s.into());
        self
    }
    pub fn with_contact_uri(mut self, s: impl Into<String>) -> Self {
        self.contact_uri = Some(s.into());
        self
    }
    pub fn with_expires(mut self, secs: u32) -> Self {
        self.expires = secs;
        self
    }
    pub fn with_accept(mut self, ct: impl Into<String>) -> Self {
        self.accept = Some(ct.into());
        self
    }
    pub fn with_credentials(mut self, c: Credentials) -> Self {
        self.credentials = Some(c);
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
        crate::api::dialog_subscription::DialogSubscriptionHandle::from_subscription(
            handle, target,
        )
    }

    pub async fn send(mut self) -> Result<SubscriptionHandle> {
        let from_uri = self
            .from_uri
            .clone()
            .unwrap_or_else(|| self.coord.config_local_uri());
        let authorization = self
            .credentials
            .as_ref()
            .map(|c| format!("Digest username=\"{}\"", c.username));
        let extra_headers = take_staged(&mut self.state);
        let opts = rvoip_sip_dialog::api::unified::SubscribeRequestOptions {
            event: self.event_package.clone(),
            expires: self.expires,
            accept: self.accept,
            from_uri: Some(from_uri),
            contact_uri: self.contact_uri,
            authorization,
            refresh: false,
            extra_headers,
        };
        // Initial SUBSCRIBE is out-of-dialog with a synchronous response
        // (200 / 4xx) — the call to dialog-core awaits the final
        // response inline. The state-machine dispatch path is only
        // appropriate once a dialog is established, so for the initial
        // send we keep the direct adapter call. The refresh path
        // (SubscribeRefreshBuilder) operates inside the dialog and
        // routes through `stage_outbound_options` + the
        // `SendOutboundSubscribe` event.
        let response = self
            .coord
            .dialog_adapter()
            .send_subscribe_oob_with_options(&self.target, opts)
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

pub struct SubscribeRefreshBuilder {
    handle: SubscriptionHandle,
    expires: Option<u32>,
    credentials: Option<Credentials>,
    state: BuilderHeaderState,
}

impl SubscribeRefreshBuilder {
    fn new(handle: SubscriptionHandle) -> Self {
        Self {
            handle,
            expires: None,
            credentials: None,
            state: BuilderHeaderState::default(),
        }
    }

    pub fn with_expires(mut self, secs: u32) -> Self {
        self.expires = Some(secs);
        self
    }
    pub fn with_credentials(mut self, c: Credentials) -> Self {
        self.credentials = Some(c);
        self
    }

    pub async fn send(mut self) -> Result<()> {
        let coord = self.handle.coord.clone().ok_or_else(|| {
            crate::errors::SessionError::InternalError(
                "SubscribeRefreshBuilder.send(): subscription handle is detached \
                 (no coordinator) — only handles returned by SubscribeBuilder.send() \
                 can be refreshed"
                    .to_string(),
            )
        })?;
        let session_id = self.handle.session_id.clone().ok_or_else(|| {
            crate::errors::SessionError::InternalError(
                "SubscribeRefreshBuilder.send(): subscription handle has no session id"
                    .to_string(),
            )
        })?;
        let authorization = self
            .credentials
            .as_ref()
            .map(|c| format!("Digest username=\"{}\"", c.username));
        let extra_headers = take_staged(&mut self.state);
        let opts = Arc::new(rvoip_sip_dialog::api::unified::SubscribeRequestOptions {
            event: self.handle.event_package.clone(),
            // Spec §7.1: refresh reuses the original interval unless the
            // caller overrides; default to 3600 if neither is provided.
            expires: self.expires.unwrap_or(3600),
            accept: None,
            from_uri: None,
            contact_uri: None,
            authorization,
            refresh: true,
            extra_headers,
        });
        coord
            .stage_outbound_options(
                &session_id,
                crate::state_machine::executor::PendingOptionsSlot::Subscribe(opts),
            )
            .await?;
        coord
            .dispatch_outbound(
                &session_id,
                crate::state_table::EventType::SendOutboundSubscribe,
            )
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
