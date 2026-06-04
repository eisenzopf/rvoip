//! `RegisterResponseBuilder` — SIP_API_DESIGN_2 §3.4.
//!
//! Composes a REGISTER response (200 OK / 401 / 407 / 423 / generic
//! 4xx-6xx) for an inbound REGISTER request. Stamps the RFC 3327
//! `Path`, RFC 3608 `Service-Route`, RFC 3455 `P-Associated-URI`,
//! RFC 3261 `Min-Expires`, and application-staged extras onto the
//! response, then publishes a `SessionToDialogEvent::SendRegisterResponse`
//! that rvoip-sip-dialog's `event_hub` consumes to send the response on the
//! wire.

use std::sync::Arc;

use rvoip_sip_core::types::headers::TypedHeader;
use rvoip_sip_core::types::Method;

use crate::api::headers::{take_staged, BuilderHeaderState, SipRequestOptions};
use crate::api::respond::AuthScheme;
use crate::api::unified::UnifiedCoordinator;
use crate::errors::{Result, SessionError};

/// What the builder ultimately emits — 200 OK, 401 / 407 challenge,
/// or a generic non-2xx reject.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum RegisterResponseKind {
    Accept,
    Challenge(AuthScheme, bool /* proxy */),
    Reject(u16),
}

/// Builds and sends a REGISTER response — 200 OK, 401/407 challenge,
/// 423 Interval Too Brief, or a generic non-2xx reject.
pub struct RegisterResponseBuilder {
    transaction_id: String,
    coordinator: Option<Arc<UnifiedCoordinator>>,
    kind: RegisterResponseKind,
    expires: u32,
    min_expires: Option<u32>,
    service_route: Vec<String>,
    path_echo: bool,
    associated_uri: Vec<String>,
    contact: Option<String>,
    /// Pre-rendered `WWW-Authenticate` / `Proxy-Authenticate` value
    /// when this builder is a challenge.
    www_authenticate_raw: Option<String>,
    /// Digest challenge parameters (only relevant when `kind` is
    /// `Challenge(Digest, _)`).
    challenge_realm: Option<String>,
    challenge_nonce: Option<String>,
    challenge_algorithm: Option<String>,
    challenge_opaque: Option<String>,
    challenge_qop: Option<String>,
    challenge_stale: bool,
    state: BuilderHeaderState,
}

impl RegisterResponseBuilder {
    pub(crate) fn new(
        transaction_id: impl Into<String>,
        coordinator: Option<Arc<UnifiedCoordinator>>,
    ) -> Self {
        Self {
            transaction_id: transaction_id.into(),
            coordinator,
            kind: RegisterResponseKind::Accept,
            expires: 3600,
            min_expires: None,
            service_route: Vec::new(),
            path_echo: false,
            associated_uri: Vec::new(),
            contact: None,
            www_authenticate_raw: None,
            challenge_realm: None,
            challenge_nonce: None,
            challenge_algorithm: None,
            challenge_opaque: None,
            challenge_qop: None,
            challenge_stale: false,
            state: BuilderHeaderState::default(),
        }
    }

    pub(crate) fn new_challenge(
        transaction_id: impl Into<String>,
        coordinator: Option<Arc<UnifiedCoordinator>>,
        scheme: AuthScheme,
    ) -> Self {
        let mut s = Self::new(transaction_id, coordinator);
        s.kind = RegisterResponseKind::Challenge(scheme, false);
        s
    }

    pub(crate) fn new_reject(
        transaction_id: impl Into<String>,
        coordinator: Option<Arc<UnifiedCoordinator>>,
        status: u16,
    ) -> Self {
        let mut s = Self::new(transaction_id, coordinator);
        s.kind = RegisterResponseKind::Reject(status);
        s
    }

    /// Set the granted registration lifetime, in seconds, stamped as
    /// `Expires` on the 200 OK.
    pub fn with_expires(mut self, secs: u32) -> Self {
        self.expires = secs;
        self
    }

    /// RFC 3261 §20.23 — `Min-Expires` for 423 Interval Too Brief.
    pub fn with_min_expires(mut self, secs: u32) -> Self {
        self.min_expires = Some(secs);
        self
    }

    /// RFC 3608 — Service-Route URIs the UA should pre-load on
    /// out-of-dialog requests within this registration binding.
    pub fn with_service_route(mut self, routes: Vec<String>) -> Self {
        self.service_route = routes;
        self
    }

    /// RFC 3327 — echo the inbound `Path:` headers on the 2xx so
    /// subsequent re-targeted requests reach the UA via the same
    /// waypoints (typical for SBC-fronted REGISTER).
    pub fn with_path_echo(mut self) -> Self {
        self.path_echo = true;
        self
    }

    /// RFC 3455 — P-Associated-URI list the registrar has provisioned
    /// for this subscriber.
    pub fn with_associated_uri(mut self, uris: Vec<String>) -> Self {
        self.associated_uri = uris;
        self
    }

    /// Override the `Contact:` URI in the 200 OK. By default the
    /// inbound request's Contact is echoed.
    pub fn with_contact_from_binding(mut self, contact_uri: impl Into<String>) -> Self {
        self.contact = Some(contact_uri.into());
        self
    }

    /// 423 / generic-reject path: change the status code on a builder
    /// that started as `accept_builder()` or `challenge_builder(..)`.
    pub fn with_status(mut self, code: u16) -> Self {
        self.kind = RegisterResponseKind::Reject(code);
        self
    }

    // ─── Auth-challenge convenience setters (mirror AuthChallengeBuilder) ───

    /// Set the challenge `realm` parameter.
    pub fn with_realm(mut self, s: impl Into<String>) -> Self {
        self.challenge_realm = Some(s.into());
        self
    }
    /// Set the challenge `nonce` parameter.
    pub fn with_nonce(mut self, s: impl Into<String>) -> Self {
        self.challenge_nonce = Some(s.into());
        self
    }
    /// Set the Digest `algorithm` parameter (e.g. MD5, SHA-256).
    pub fn with_algorithm(mut self, s: impl Into<String>) -> Self {
        self.challenge_algorithm = Some(s.into());
        self
    }
    /// Set the Digest `opaque` parameter.
    pub fn with_opaque(mut self, s: impl Into<String>) -> Self {
        self.challenge_opaque = Some(s.into());
        self
    }
    /// Set the Digest `qop` parameter (e.g. `auth`, `auth-int`).
    pub fn with_qop(mut self, s: impl Into<String>) -> Self {
        self.challenge_qop = Some(s.into());
        self
    }
    /// Set the Digest `stale` flag (request re-auth with a fresh nonce).
    pub fn with_stale(mut self, stale: bool) -> Self {
        self.challenge_stale = stale;
        self
    }

    /// Set Digest challenge parameters from `rvoip-auth-core`.
    pub fn with_digest_challenge(mut self, challenge: &crate::auth::DigestChallenge) -> Self {
        self.challenge_realm = Some(challenge.realm.clone());
        self.challenge_nonce = Some(challenge.nonce.clone());
        self.challenge_algorithm = Some(challenge.algorithm.as_str().to_string());
        self.challenge_opaque = challenge.opaque.clone();
        self.challenge_qop = challenge.qop.as_ref().map(|qop| qop.join(","));
        self
    }

    /// Mark this challenge as a proxy challenge (407 instead of 401).
    pub fn as_proxy_challenge(mut self, proxy: bool) -> Self {
        if let RegisterResponseKind::Challenge(scheme, _) = self.kind {
            self.kind = RegisterResponseKind::Challenge(scheme, proxy);
        }
        self
    }

    /// Pre-rendered `WWW-Authenticate:` body — for callers that
    /// compute the challenge themselves via auth-core.
    pub fn with_raw_www_authenticate(mut self, body: impl Into<String>) -> Self {
        self.www_authenticate_raw = Some(body.into());
        self
    }

    /// Render the digest challenge body for the wire when the builder
    /// is in `Challenge(Digest, _)` mode.
    fn render_digest_challenge(&self) -> Result<String> {
        let realm = self.challenge_realm.clone().ok_or_else(|| {
            SessionError::InvalidInput(
                "RegisterResponseBuilder challenge requires with_realm(..)".to_string(),
            )
        })?;
        let nonce = self.challenge_nonce.clone().ok_or_else(|| {
            SessionError::InvalidInput(
                "RegisterResponseBuilder challenge requires with_nonce(..)".to_string(),
            )
        })?;
        let mut out = format!("Digest realm=\"{}\", nonce=\"{}\"", realm, nonce);
        if let Some(alg) = self.challenge_algorithm.as_ref() {
            out.push_str(&format!(", algorithm={}", alg));
        }
        if let Some(opaque) = self.challenge_opaque.as_ref() {
            out.push_str(&format!(", opaque=\"{}\"", opaque));
        }
        if let Some(qop) = self.challenge_qop.as_ref() {
            out.push_str(&format!(", qop=\"{}\"", qop));
        }
        if self.challenge_stale {
            out.push_str(", stale=true");
        }
        Ok(out)
    }

    /// Build the cross-crate `SendRegisterResponse` event payload
    /// fields without sending. Useful for tests and for the registrar
    /// crate's migration path.
    pub fn build_event_fields(mut self) -> Result<RegisterResponseEventFields> {
        let extras = take_staged(&mut self.state);
        let extra_headers_wire = extras
            .into_iter()
            .map(|h| {
                let name = format!("{:?}", h.name());
                let value = render_typed_header_value(&h);
                (name, value)
            })
            .collect::<Vec<_>>();

        let (status_code, reason, www_authenticate) = match self.kind {
            RegisterResponseKind::Accept => (200u16, "OK".to_string(), None),
            RegisterResponseKind::Reject(s) => (s, default_reason_for(s).to_string(), None),
            RegisterResponseKind::Challenge(AuthScheme::Digest, proxy) => {
                let rendered = self
                    .www_authenticate_raw
                    .clone()
                    .map(Ok)
                    .unwrap_or_else(|| self.render_digest_challenge())?;
                if proxy {
                    (
                        407u16,
                        "Proxy Authentication Required".to_string(),
                        Some(rendered),
                    )
                } else {
                    (401u16, "Unauthorized".to_string(), Some(rendered))
                }
            }
            RegisterResponseKind::Challenge(AuthScheme::Bearer, proxy) => {
                let realm = self.challenge_realm.clone().ok_or_else(|| {
                    SessionError::InvalidInput(
                        "Bearer challenge requires with_realm(..)".to_string(),
                    )
                })?;
                let rendered = format!("Bearer realm=\"{}\"", realm);
                if proxy {
                    (
                        407u16,
                        "Proxy Authentication Required".to_string(),
                        Some(rendered),
                    )
                } else {
                    (401u16, "Unauthorized".to_string(), Some(rendered))
                }
            }
            RegisterResponseKind::Challenge(AuthScheme::Basic, proxy) => {
                let realm = self.challenge_realm.clone().ok_or_else(|| {
                    SessionError::InvalidInput(
                        "Basic challenge requires with_realm(..)".to_string(),
                    )
                })?;
                let rendered = format!("Basic realm=\"{}\"", realm);
                if proxy {
                    (
                        407u16,
                        "Proxy Authentication Required".to_string(),
                        Some(rendered),
                    )
                } else {
                    (401u16, "Unauthorized".to_string(), Some(rendered))
                }
            }
            RegisterResponseKind::Challenge(AuthScheme::Aka, proxy) => {
                let realm = self.challenge_realm.clone().ok_or_else(|| {
                    SessionError::InvalidInput("AKA challenge requires with_realm(..)".to_string())
                })?;
                let nonce = self.challenge_nonce.clone().ok_or_else(|| {
                    SessionError::InvalidInput("AKA challenge requires with_nonce(..)".to_string())
                })?;
                let algorithm = self
                    .challenge_algorithm
                    .clone()
                    .unwrap_or_else(|| "AKAv1-MD5".to_string());
                let mut rendered = format!(
                    "Digest realm=\"{}\", nonce=\"{}\", algorithm={}",
                    realm, nonce, algorithm
                );
                if let Some(qop) = self.challenge_qop.as_ref() {
                    rendered.push_str(&format!(", qop=\"{}\"", qop));
                }
                if self.challenge_stale {
                    rendered.push_str(", stale=true");
                }
                if proxy {
                    (
                        407u16,
                        "Proxy Authentication Required".to_string(),
                        Some(rendered),
                    )
                } else {
                    (401u16, "Unauthorized".to_string(), Some(rendered))
                }
            }
        };

        Ok(RegisterResponseEventFields {
            transaction_id: self.transaction_id,
            status_code,
            reason,
            www_authenticate,
            contact: self.contact,
            expires: if matches!(self.kind, RegisterResponseKind::Accept) {
                Some(self.expires)
            } else {
                None
            },
            min_expires: self.min_expires,
            service_route: self.service_route,
            path_echo: self.path_echo,
            associated_uri: self.associated_uri,
            extra_headers: extra_headers_wire,
        })
    }

    /// Publish the response via `SessionToDialogEvent::SendRegisterResponse`.
    /// Requires the builder to have been constructed from an
    /// `IncomingRegister` that carries a coordinator handle; otherwise
    /// returns `Err(InvalidInput)` so test / synthesized paths get a
    /// clear error.
    pub async fn send(self) -> Result<()> {
        let coordinator = self.coordinator.clone().ok_or_else(|| {
            SessionError::InvalidInput(
                "RegisterResponseBuilder.send() requires an IncomingRegister with a \
                 coordinator hook; synthesized wrappers (tests, legacy registrar) cannot \
                 dispatch via this builder"
                    .to_string(),
            )
        })?;

        let fields = self.build_event_fields()?;
        let event = rvoip_infra_common::events::cross_crate::RvoipCrossCrateEvent::SessionToDialog(
            rvoip_infra_common::events::cross_crate::SessionToDialogEvent::SendRegisterResponse {
                transaction_id: fields.transaction_id,
                status_code: fields.status_code,
                reason: fields.reason,
                www_authenticate: fields.www_authenticate,
                contact: fields.contact,
                expires: fields.expires,
                min_expires: fields.min_expires,
                service_route: fields.service_route,
                path_echo: fields.path_echo,
                associated_uri: fields.associated_uri,
                extra_headers: fields.extra_headers,
            },
        );

        coordinator
            .global_coordinator
            .publish(std::sync::Arc::new(event))
            .await
            .map_err(|e| {
                SessionError::DialogError(format!("Failed to publish REGISTER response: {}", e))
            })?;

        Ok(())
    }
}

/// SIP_API_DESIGN_2 Phase D — wire-format snapshot of every field a
/// `RegisterResponseBuilder` would publish. Returned by
/// `build_event_fields()` for test inspection.
#[derive(Debug, Clone)]
pub struct RegisterResponseEventFields {
    /// Transaction the response belongs to.
    pub transaction_id: String,
    /// SIP status code of the response.
    pub status_code: u16,
    /// Reason phrase of the response.
    pub reason: String,
    /// Rendered `WWW-Authenticate` / `Proxy-Authenticate` body, when a
    /// challenge.
    pub www_authenticate: Option<String>,
    /// `Contact` URI for the 200 OK, when overridden.
    pub contact: Option<String>,
    /// Granted `Expires` lifetime in seconds (2xx only).
    pub expires: Option<u32>,
    /// `Min-Expires` value for a 423 response.
    pub min_expires: Option<u32>,
    /// RFC 3608 `Service-Route` URIs to stamp on the response.
    pub service_route: Vec<String>,
    /// Whether to echo the inbound RFC 3327 `Path` headers.
    pub path_echo: bool,
    /// RFC 3455 `P-Associated-URI` list.
    pub associated_uri: Vec<String>,
    /// Application-staged extra headers, as `(name, value)` wire pairs.
    pub extra_headers: Vec<(String, String)>,
}

impl SipRequestOptions for RegisterResponseBuilder {
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

fn render_typed_header_value(h: &TypedHeader) -> String {
    // For `TypedHeader::Other(_, HeaderValue::Raw(bytes))` we have the
    // raw on-wire bytes already. For typed variants, defer to the
    // Display impl which renders the RFC 3261-compliant value.
    if let TypedHeader::Other(_, hv) = h {
        if let Some(s) = hv.as_text() {
            return s.to_string();
        }
    }
    // Fall back to the Display impl, which renders "Name: value" — we
    // strip the prefix so the wire emitter doesn't double-stamp the
    // header name.
    let rendered = h.to_string();
    if let Some(idx) = rendered.find(':') {
        rendered[idx + 1..].trim().to_string()
    } else {
        rendered
    }
}

fn default_reason_for(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        401 => "Unauthorized",
        403 => "Forbidden",
        404 => "Not Found",
        407 => "Proxy Authentication Required",
        423 => "Interval Too Brief",
        503 => "Service Unavailable",
        _ => "Rejected",
    }
}
