//! `OutboundCallBuilder` — SIP_API_DESIGN_2 §3.3 INVITE builder.

use std::sync::Arc;

use rvoip_sip_core::types::Method;

use crate::api::handle::CallId;
use crate::api::headers::{BuilderHeaderState, SipRequestOptions};
use crate::api::unified::UnifiedCoordinator;
use crate::auth::SipClientAuth;
use crate::errors::Result;
use crate::types::Credentials;

/// Per-request override for the `P-Asserted-Identity` (RFC 3325).
#[non_exhaustive]
#[derive(Default, Debug, Clone)]
pub enum PaiOverride {
    /// Inherit `Config.pai_uri`.
    #[default]
    Default,
    /// Suppress PAI emission even if `Config` has one.
    Suppress,
    /// Override `Config.pai_uri` for this call only.
    Use(String),
}

/// Per-request override for the outbound proxy `Route:` header.
#[non_exhaustive]
#[derive(Default, Debug, Clone)]
pub enum ProxyOverride {
    /// Inherit `Config.outbound_proxy_uri`.
    #[default]
    Default,
    /// Suppress the outbound proxy `Route:` even if `Config` has one.
    Suppress,
    /// Override the outbound proxy `Route:` for this call only.
    Use(String),
}

/// SIP_API_DESIGN_2 §7.1 — frozen snapshot of an `OutboundCallBuilder`
/// staged on `SessionState.pending_invite_options` and consumed by the
/// `Action::SendINVITEWithOptions` handler.
///
/// `OutboundCallOptions` is an rvoip-sip-side struct (not in
/// rvoip-sip-dialog) because INVITE carries rvoip-sip concerns rvoip-sip-dialog
/// doesn't need: PAI mode, credentials, transfer-leg tracking,
/// `supported_100rel`. The state machine unpacks it at the
/// DialogAdapter boundary and calls rvoip-sip-dialog's existing
/// `make_call_with_extra_headers_for_session`.
#[derive(Default, Debug, Clone)]
pub struct OutboundCallOptionsSnapshot {
    /// `From:` URI; falls back to `Config.local_uri` when `None`.
    pub from: Option<String>,
    /// Request-URI / `To:` target of the INVITE.
    pub to: String,
    /// SDP offer body, if any.
    pub sdp: Option<String>,
    /// Digest credentials used for 401/407 retry.
    pub credentials: Option<Credentials>,
    /// General SIP auth used for 401/407 retry.
    pub auth: Option<SipClientAuth>,
    /// `P-Asserted-Identity` (RFC 3325) override mode for this call.
    pub pai_override: PaiOverride,
    /// `Contact:` URI override advertised on the INVITE.
    pub contact_uri: Option<String>,
    /// Outbound proxy `Route:` override mode for this call.
    pub outbound_proxy_override: ProxyOverride,
    /// `Subject:` header value.
    pub subject: Option<String>,
    /// `From:` display name override.
    pub from_display: Option<String>,
    /// Pre-computed `Authorization:` header value, bypassing 401-driven
    /// digest computation.
    pub precomputed_auth: Option<String>,
    /// When set, marks this INVITE as the B leg of an attended transfer
    /// initiated by the named transferor session.
    pub transfer_leg: Option<CallId>,
    /// Whether RFC 3262 reliable provisional responses are advertised.
    pub supported_100rel: bool,
    /// Application-staged extra headers appended after stack-managed ones.
    pub extra_headers: Vec<rvoip_sip_core::types::TypedHeader>,
    /// When true, the outbound INVITE applies SBC topology hiding:
    /// stack-managed Via headers below the top entry are stripped
    /// and Record-Route entries not pointing at this SBC are
    /// removed before send. Default `false` — applications that
    /// want B2BUA-style hiding turn this on per call via
    /// [`OutboundCallBuilder::with_topology_hiding`].
    pub topology_hiding: bool,
}

/// Outbound INVITE builder.
pub struct OutboundCallBuilder {
    coord: Arc<UnifiedCoordinator>,
    from: Option<String>,
    to: String,
    sdp: Option<String>,
    credentials: Option<Credentials>,
    auth: Option<SipClientAuth>,
    pai: PaiOverride,
    contact_uri: Option<String>,
    outbound_proxy: ProxyOverride,
    subject: Option<String>,
    from_display: Option<String>,
    precomputed_authorization: Option<String>,
    transfer_leg: Option<CallId>,
    supported_100rel: bool,
    state: BuilderHeaderState,
    topology_hiding: bool,
}

impl OutboundCallBuilder {
    pub(crate) fn new(
        coord: Arc<UnifiedCoordinator>,
        from: Option<String>,
        to: impl Into<String>,
    ) -> Self {
        Self {
            coord,
            from,
            to: to.into(),
            sdp: None,
            credentials: None,
            auth: None,
            pai: PaiOverride::default(),
            contact_uri: None,
            outbound_proxy: ProxyOverride::default(),
            subject: None,
            from_display: None,
            precomputed_authorization: None,
            transfer_leg: None,
            supported_100rel: false,
            state: BuilderHeaderState::default(),
            topology_hiding: false,
        }
    }

    /// Attach an SDP offer.
    pub fn with_sdp(mut self, sdp: impl Into<String>) -> Self {
        self.sdp = Some(sdp.into());
        self
    }

    /// Attach Digest credentials for UAC 401/407 retry.
    pub fn with_credentials(mut self, creds: Credentials) -> Self {
        self.credentials = Some(creds);
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

    /// Override the `P-Asserted-Identity` URI for this call only.
    pub fn with_pai(mut self, uri: impl Into<String>) -> Self {
        self.pai = PaiOverride::Use(uri.into());
        self
    }

    /// Suppress `P-Asserted-Identity` emission even when
    /// `Config.pai_uri` is set.
    pub fn without_pai(mut self) -> Self {
        self.pai = PaiOverride::Suppress;
        self
    }

    /// Override the `Contact:` URI advertised on this INVITE.
    pub fn with_contact_uri(mut self, uri: impl Into<String>) -> Self {
        self.contact_uri = Some(uri.into());
        self
    }

    /// Override the outbound proxy `Route:` for this call only.
    pub fn with_outbound_proxy(mut self, uri: impl Into<String>) -> Self {
        self.outbound_proxy = ProxyOverride::Use(uri.into());
        self
    }

    /// Suppress the outbound proxy `Route:` even when
    /// `Config.outbound_proxy_uri` is set.
    pub fn without_outbound_proxy(mut self) -> Self {
        self.outbound_proxy = ProxyOverride::Suppress;
        self
    }

    /// Attach a `Subject:` header.
    pub fn with_subject(mut self, subject: impl Into<String>) -> Self {
        self.subject = Some(subject.into());
        self
    }

    /// Override the `From:` display name.
    pub fn with_from_display(mut self, display: impl Into<String>) -> Self {
        self.from_display = Some(display.into());
        self
    }

    /// Pre-computed `Authorization:` header value — bypasses
    /// 401-driven digest computation.
    pub fn with_precomputed_authorization(mut self, value: impl Into<String>) -> Self {
        self.precomputed_authorization = Some(value.into());
        self
    }

    /// Mark this INVITE as the B leg of a `transferor`-initiated
    /// attended transfer (used for media bridging + REFER-completion
    /// NOTIFY).
    pub fn as_transfer_leg(mut self, transferor: &CallId) -> Self {
        self.transfer_leg = Some(transferor.clone());
        self
    }

    /// Advertise RFC 3262 reliable provisional support.
    pub fn with_supported_100rel(mut self, supported: bool) -> Self {
        self.supported_100rel = supported;
        self
    }

    /// Apply SBC topology hiding to this outbound INVITE. When enabled,
    /// any Via headers below the SBC's own top Via are stripped, and
    /// Record-Route entries that do not point at this SBC are removed
    /// before the message reaches the wire. Use this on B2BUA forwards
    /// where the inbound topology (upstream Via stack, intermediate
    /// Record-Routes) must not leak to downstream peers.
    ///
    /// The default outbound-INVITE shape already builds a fresh Via
    /// stack and Contact, so most call sites do not need this — the
    /// flag matters only for forward paths that explicitly carry
    /// inherited topology headers across (e.g. proxy-style B2BUA on
    /// top of `Transport::send_message_raw`).
    pub fn with_topology_hiding(mut self, enabled: bool) -> Self {
        self.topology_hiding = enabled;
        self
    }

    /// Send the INVITE.
    ///
    /// Routes through the unified state-machine path: creates the
    /// session, stages an [`OutboundCallOptionsSnapshot`] on the
    /// session's INVITE stash, then dispatches
    /// [`EventType::SendOutboundInvite`](crate::state_table::EventType::SendOutboundInvite).
    /// The state table's `(Idle, SendOutboundInvite, UAC)` row runs
    /// `CreateDialog → CreateMediaSession → GenerateLocalSDP →
    /// SendINVITEWithOptions`, which drains the stash and emits the
    /// INVITE through rvoip-sip-dialog's `send_invite_with_extra_headers`.
    /// Application-staged headers, PAI override, outbound-proxy
    /// override, credentials and Subject ride through the snapshot.
    pub async fn send(self) -> Result<CallId> {
        let from = self
            .from
            .clone()
            .unwrap_or_else(|| self.coord.config_local_uri());
        let to = self.to.clone();

        // Resolve PAI per the builder's override mode against Config.
        let pai_uri = match &self.pai {
            PaiOverride::Use(uri) => Some(uri.clone()),
            PaiOverride::Suppress => None,
            PaiOverride::Default => self.coord.config_pai_uri(),
        };

        // Fall back to `Config::credentials` when the application did
        // not stage per-call credentials, so PBX-auth flows that only
        // configure peer-level credentials keep working.
        let credentials = self
            .credentials
            .clone()
            .or_else(|| self.coord.config_credentials());
        let auth = self
            .auth
            .clone()
            .or_else(|| self.coord.config_auth())
            .or_else(|| credentials.clone().map(Into::into));

        // Build the snapshot — folds every override into a frozen
        // struct that the state-machine handler reads back verbatim.
        let snapshot = std::sync::Arc::new(OutboundCallOptionsSnapshot {
            from: Some(from.clone()),
            to: to.clone(),
            sdp: self.sdp,
            credentials,
            auth,
            pai_override: self.pai,
            contact_uri: self.contact_uri,
            outbound_proxy_override: self.outbound_proxy,
            subject: self.subject,
            from_display: self.from_display,
            precomputed_auth: self.precomputed_authorization,
            transfer_leg: self.transfer_leg,
            supported_100rel: self.supported_100rel,
            extra_headers: self.state.headers.clone(),
            topology_hiding: self.topology_hiding,
        });

        // Create the session up front — Idle UAC. Then mirror
        // `make_call_inner`'s pre-event field plumbing so a fast
        // loopback 180 Ringing can't beat our state update: credentials,
        // PAI, transfer leg, extra headers land on SessionState before
        // the event enters the machine. The state-table `CreateDialog`
        // action picks them up.
        let session_id = crate::state_table::SessionId::new();
        self.coord
            .helpers
            .create_session(
                session_id.clone(),
                from.clone(),
                to.clone(),
                crate::state_table::Role::UAC,
            )
            .await?;

        if snapshot.credentials.is_some()
            || snapshot.auth.is_some()
            || pai_uri.is_some()
            || snapshot.transfer_leg.is_some()
            || !snapshot.extra_headers.is_empty()
        {
            let mut session = self.coord.session_state(&session_id).await?;
            if let Some(c) = snapshot.credentials.clone() {
                session.credentials = Some(c);
            }
            if let Some(auth) = snapshot.auth.clone() {
                session.auth = Some(auth);
            }
            if let Some(pai) = pai_uri {
                session.pai_uri = Some(pai);
            }
            if let Some(transferor) = snapshot.transfer_leg.clone() {
                session.transferor_session_id = Some(transferor);
                session.is_transfer_call = true;
            }
            if !snapshot.extra_headers.is_empty() {
                session.extra_headers = snapshot.extra_headers.clone();
            }
            self.coord
                .helpers
                .state_machine
                .store
                .update_session(session)
                .await?;
        }

        self.coord
            .stage_outbound_options(
                &session_id,
                crate::state_machine::executor::PendingOptionsSlot::Invite(snapshot),
            )
            .await?;
        self.coord
            .dispatch_outbound(
                &session_id,
                crate::state_table::EventType::SendOutboundInvite,
            )
            .await?;
        self.coord
            .schedule_outbound_setup_timeout(&session_id)
            .await;
        Ok(session_id)
    }
}

impl SipRequestOptions for OutboundCallBuilder {
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
