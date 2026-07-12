//! G2 — Pluggable authentication hooks for WHIP and WebSocket signaling.
//!
//! Production deployments register their own [`WhipAuthHook`] /
//! [`WsAuthHook`] (typically a Bearer-token validator backed by a JWT
//! library). The default registered when neither is supplied is
//! [`AnonymousAuth`], which accepts every request — backward-compatible
//! with the H1–H7 surface.
//!
//! Per RFC 9725 §4.1 production WHIP servers MUST accept
//! `Authorization: Bearer ...`. The `WhipAuthHook` trait surfaces that
//! header (plus the request method, path, and peer address) to the
//! deployment's policy code; the hook returns an [`AuthContext`] converted
//! into adapter-owned route authorization (retaining the complete principal),
//! or an [`AuthRejection`] mapped to a 401/403/429.

use std::fmt;
use std::net::SocketAddr;

use async_trait::async_trait;
use rvoip_auth_core::{AuthenticatedPrincipal, BearerAuthError, BearerValidator};

use crate::adapter::RouteAuthorization;

const MAX_SESSION_HINT_PREFIX_BYTES: usize = 64;
const MAX_SESSION_HINT_BYTES: usize = rvoip_core::MAX_INBOUND_ROUTING_HINT_BYTES;
const AUTH_TOKEN_SUBPROTOCOL_PREFIX: &str = "token.";
const SIGNALING_SUBPROTOCOL: &str = "rvoip.webrtc.v1";

/// Authentication context attached to a request after a successful hook.
/// Its complete principal is retained on the adapter route for ownership,
/// orchestration, identity verification, and audit events.
#[derive(Clone)]
pub struct AuthContext {
    /// Opaque tenant / user identifier — meaning is deployment-defined.
    pub subject: String,
    /// Scopes granted (e.g. `["whip:publish", "whep:subscribe"]`).
    pub scopes: Vec<String>,
    /// Optional hint about which session the caller is acting on
    /// (extracted from the URL path or a custom header).
    pub session_hint: Option<String>,
    /// Full validated principal retained for ownership and tenant policy.
    pub principal: Option<AuthenticatedPrincipal>,
}

impl fmt::Debug for AuthContext {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter
            .debug_struct("AuthContext")
            .field("subject", &"[redacted]")
            .field("scope_count", &self.scopes.len())
            .field("has_session_hint", &self.session_hint.is_some())
            .field("has_principal", &self.principal.is_some())
            .finish()
    }
}

impl AuthContext {
    pub fn anonymous() -> Self {
        Self {
            subject: "anonymous".to_string(),
            scopes: Vec::new(),
            session_hint: None,
            principal: None,
        }
    }

    pub fn has_scope(&self, scope: &str) -> bool {
        self.scopes.iter().any(|s| s == scope)
    }

    /// Convert request authentication into the adapter-owned route boundary.
    /// Complete principals use issuer + tenant + subject; legacy hooks retain
    /// their historical subject semantics, while anonymous mode remains
    /// compatible with authentication-disabled deployments.
    pub(crate) fn route_authorization(&self) -> RouteAuthorization {
        match self.principal.clone() {
            Some(principal) => RouteAuthorization::principal(principal),
            None if self.subject == "anonymous" => RouteAuthorization::anonymous(),
            None => RouteAuthorization::legacy_subject(self.subject.clone()),
        }
    }
}

/// Reasons the auth hook rejected a request.
#[derive(Clone, Debug)]
pub enum AuthRejection {
    /// 401 Unauthorized. The contained string is rendered into
    /// `WWW-Authenticate` (per RFC 7235 §4.1) — typically
    /// `Bearer realm="rvoip"`.
    Unauthorized { www_authenticate: String },
    /// 403 Forbidden — authenticated but lacks the required scope.
    Forbidden,
    /// 429 Too Many Requests with `Retry-After: <seconds>`.
    Throttled { retry_after_secs: u32 },
}

/// WHIP / WHEP HTTP authentication hook (RFC 9725 §4.1).
///
/// Default implementation: [`AnonymousAuth`] (accepts everything).
#[async_trait]
pub trait WhipAuthHook: Send + Sync {
    /// `method` is the canonical HTTP method name ("POST", "PATCH",
    /// "DELETE", "OPTIONS"). `path` is the request path.
    async fn authenticate(
        &self,
        method: &str,
        path: &str,
        bearer: Option<&str>,
        peer_addr: SocketAddr,
    ) -> Result<AuthContext, AuthRejection>;
}

/// WebSocket signaling authentication hook.
///
/// Tokens may arrive via `Sec-WebSocket-Protocol` (some clients smuggle
/// a token after the version tag, e.g.
/// `rvoip.webrtc.v1, token.<base64>`) or via a `?access_token=…` query
/// parameter; both are surfaced here.
#[async_trait]
pub trait WsAuthHook: Send + Sync {
    async fn authenticate(
        &self,
        subprotocols: &[String],
        query_token: Option<&str>,
        peer_addr: SocketAddr,
    ) -> Result<AuthContext, AuthRejection>;

    /// Whether a requested subprotocol contains private routing or credential
    /// material and therefore must never be echoed in the upgrade response.
    fn subprotocol_is_private(&self, value: &str) -> bool {
        value.starts_with(AUTH_TOKEN_SUBPROTOCOL_PREFIX)
    }
}

/// No-op hook — every request is accepted as the anonymous principal.
/// This is the default when no hook is registered, preserving the
/// pre-G2 behavior.
pub struct AnonymousAuth;

#[async_trait]
impl WhipAuthHook for AnonymousAuth {
    async fn authenticate(
        &self,
        _method: &str,
        _path: &str,
        _bearer: Option<&str>,
        _peer_addr: SocketAddr,
    ) -> Result<AuthContext, AuthRejection> {
        Ok(AuthContext::anonymous())
    }
}

#[async_trait]
impl WsAuthHook for AnonymousAuth {
    async fn authenticate(
        &self,
        _subprotocols: &[String],
        _query_token: Option<&str>,
        _peer_addr: SocketAddr,
    ) -> Result<AuthContext, AuthRejection> {
        Ok(AuthContext::anonymous())
    }
}

/// Reference implementation: a single static Bearer token (testing /
/// demo only — production should validate JWTs / OAuth tokens).
pub struct BearerStaticTokenAuth {
    pub token: String,
    pub scopes: Vec<String>,
    pub realm: String,
}

impl BearerStaticTokenAuth {
    pub fn new(token: impl Into<String>) -> Self {
        Self {
            token: token.into(),
            scopes: vec!["whip:publish".to_string(), "whep:subscribe".to_string()],
            realm: "rvoip".to_string(),
        }
    }
}

#[async_trait]
impl WhipAuthHook for BearerStaticTokenAuth {
    async fn authenticate(
        &self,
        _method: &str,
        _path: &str,
        bearer: Option<&str>,
        _peer_addr: SocketAddr,
    ) -> Result<AuthContext, AuthRejection> {
        match bearer {
            Some(t) if t == self.token => Ok(AuthContext {
                subject: "static-token-bearer".into(),
                scopes: self.scopes.clone(),
                session_hint: None,
                principal: None,
            }),
            _ => Err(AuthRejection::Unauthorized {
                www_authenticate: format!("Bearer realm=\"{}\"", self.realm),
            }),
        }
    }
}

#[async_trait]
impl WsAuthHook for BearerStaticTokenAuth {
    async fn authenticate(
        &self,
        subprotocols: &[String],
        query_token: Option<&str>,
        _peer_addr: SocketAddr,
    ) -> Result<AuthContext, AuthRejection> {
        // Look for the token in the subprotocol list (e.g.
        // `token.<value>`) or as a `?access_token=…` query param.
        let from_subproto = subprotocols
            .iter()
            .find_map(|s| s.strip_prefix("token.").map(|t| t.to_string()));
        let token = from_subproto.as_deref().or(query_token);
        match token {
            Some(t) if t == self.token => Ok(AuthContext {
                subject: "static-token-bearer".into(),
                scopes: self.scopes.clone(),
                session_hint: None,
                principal: None,
            }),
            _ => Err(AuthRejection::Unauthorized {
                www_authenticate: format!("Bearer realm=\"{}\"", self.realm),
            }),
        }
    }
}

/// First-party bridge from `rvoip-auth-core` validators into both WebRTC
/// signaling surfaces.
pub struct AuthCoreHook {
    validator: std::sync::Arc<dyn BearerValidator>,
    realm: String,
    pub allow_query_tokens: bool,
    session_hint_subprotocol_prefix: Option<String>,
}

/// Invalid opt-in WebSocket session-hint configuration.
#[derive(Clone, Copy, Debug, Eq, PartialEq, thiserror::Error)]
pub enum SessionHintConfigError {
    /// Prefixes are bounded visible WebSocket-token characters and must not
    /// collide with the authentication-token prefix.
    #[error("invalid WebSocket session-hint subprotocol prefix")]
    InvalidPrefix,
}

impl AuthCoreHook {
    pub fn new(validator: std::sync::Arc<dyn BearerValidator>) -> Self {
        Self {
            validator,
            realm: "rvoip".into(),
            allow_query_tokens: false,
            session_hint_subprotocol_prefix: None,
        }
    }

    pub fn with_realm(mut self, realm: impl Into<String>) -> Self {
        self.realm = realm.into();
        self
    }

    pub fn allow_query_tokens(mut self, allow: bool) -> Self {
        self.allow_query_tokens = allow;
        self
    }

    /// Retains a second, explicitly prefixed WebSocket subprotocol value as
    /// the adapter's single-take inbound routing hint.
    ///
    /// Authentication remains independent and still uses `token.<bearer>`.
    /// The prefix is deployment policy (for example `bridgefu.attach.`); the
    /// extracted value is redacted from `Debug` and rejected when missing,
    /// duplicated, empty, oversized, or control-bearing.
    pub fn try_with_session_hint_subprotocol_prefix(
        mut self,
        prefix: impl Into<String>,
    ) -> Result<Self, SessionHintConfigError> {
        let prefix = prefix.into();
        if prefix.is_empty()
            || prefixes_overlap(&prefix, AUTH_TOKEN_SUBPROTOCOL_PREFIX)
            || prefixes_overlap(&prefix, SIGNALING_SUBPROTOCOL)
            || prefix.len() > MAX_SESSION_HINT_PREFIX_BYTES
            || !prefix.bytes().all(is_subprotocol_prefix_byte)
        {
            return Err(SessionHintConfigError::InvalidPrefix);
        }
        self.session_hint_subprotocol_prefix = Some(prefix);
        Ok(self)
    }

    async fn validate(
        &self,
        token: Option<&str>,
        required_scope: &str,
    ) -> Result<AuthContext, AuthRejection> {
        let Some(token) = token.filter(|token| !token.is_empty()) else {
            return Err(self.unauthorized());
        };
        let principal =
            self.validator
                .validate_principal(token)
                .await
                .map_err(|error| match error {
                    BearerAuthError::Unavailable(_) => AuthRejection::Throttled {
                        retry_after_secs: 1,
                    },
                    _ => self.unauthorized(),
                })?;
        if !principal.has_scope(required_scope) {
            return Err(AuthRejection::Forbidden);
        }
        Ok(AuthContext {
            subject: principal.subject.clone(),
            scopes: principal.scopes.clone(),
            session_hint: None,
            principal: Some(principal),
        })
    }

    fn unauthorized(&self) -> AuthRejection {
        AuthRejection::Unauthorized {
            www_authenticate: format!("Bearer realm=\"{}\"", self.realm),
        }
    }

    fn session_hint(&self, subprotocols: &[String]) -> Result<Option<String>, AuthRejection> {
        let Some(prefix) = self.session_hint_subprotocol_prefix.as_deref() else {
            return Ok(None);
        };
        let mut matches = subprotocols
            .iter()
            .filter_map(|value| value.strip_prefix(prefix));
        let Some(value) = matches.next() else {
            return Err(AuthRejection::Forbidden);
        };
        if matches.next().is_some()
            || value.is_empty()
            || value.len() > MAX_SESSION_HINT_BYTES
            || value.chars().any(char::is_control)
        {
            return Err(AuthRejection::Forbidden);
        }
        Ok(Some(value.to_owned()))
    }
}

const fn is_subprotocol_prefix_byte(byte: u8) -> bool {
    byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.')
}

fn prefixes_overlap(left: &str, right: &str) -> bool {
    left.starts_with(right) || right.starts_with(left)
}

#[async_trait]
impl WhipAuthHook for AuthCoreHook {
    async fn authenticate(
        &self,
        _method: &str,
        path: &str,
        bearer: Option<&str>,
        _peer_addr: SocketAddr,
    ) -> Result<AuthContext, AuthRejection> {
        let scope = if path.starts_with("/whep") {
            "whep:subscribe"
        } else {
            "whip:publish"
        };
        self.validate(bearer, scope).await
    }
}

#[async_trait]
impl WsAuthHook for AuthCoreHook {
    async fn authenticate(
        &self,
        subprotocols: &[String],
        query_token: Option<&str>,
        _peer_addr: SocketAddr,
    ) -> Result<AuthContext, AuthRejection> {
        let mut bearer_values = subprotocols
            .iter()
            .filter_map(|value| value.strip_prefix(AUTH_TOKEN_SUBPROTOCOL_PREFIX));
        let from_subprotocol = bearer_values.next();
        if bearer_values.next().is_some()
            || (from_subprotocol.is_some() && self.allow_query_tokens && query_token.is_some())
        {
            return Err(self.unauthorized());
        }
        let token =
            from_subprotocol.or_else(|| self.allow_query_tokens.then_some(query_token).flatten());
        let mut context = self.validate(token, "webrtc:connect").await?;
        context.session_hint = self.session_hint(subprotocols)?;
        Ok(context)
    }

    fn subprotocol_is_private(&self, value: &str) -> bool {
        value.starts_with(AUTH_TOKEN_SUBPROTOCOL_PREFIX)
            || self
                .session_hint_subprotocol_prefix
                .as_deref()
                .is_some_and(|prefix| value.starts_with(prefix))
    }
}

/// Parse `Authorization: Bearer <token>` into just `<token>`. Returns
/// `None` if the header is missing, malformed, or uses a different scheme.
pub fn extract_bearer(header: Option<&str>) -> Option<&str> {
    let header = header?;
    let rest = header
        .strip_prefix("Bearer ")
        .or_else(|| header.strip_prefix("bearer "))?;
    let trimmed = rest.trim();
    if trimmed.is_empty() {
        None
    } else {
        Some(trimmed)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    struct TestPrincipalValidator;

    #[async_trait]
    impl BearerValidator for TestPrincipalValidator {
        async fn validate(
            &self,
            token: &str,
        ) -> Result<rvoip_core::IdentityAssurance, BearerAuthError> {
            if token != "valid-auth" {
                return Err(BearerAuthError::Invalid("invalid test token".into()));
            }
            Ok(rvoip_core::IdentityAssurance::Pseudonymous {
                ephemeral_key: rvoip_core::Jwk(serde_json::json!({"kty": "test"})),
            })
        }

        async fn validate_principal(
            &self,
            token: &str,
        ) -> Result<AuthenticatedPrincipal, BearerAuthError> {
            let assurance = self.validate(token).await?;
            Ok(AuthenticatedPrincipal {
                subject: "private-subject".into(),
                tenant: Some("private-tenant".into()),
                scopes: vec!["webrtc:connect".into()],
                issuer: Some("private-issuer".into()),
                expires_at: None,
                method: rvoip_auth_core::AuthenticationMethod::Jwt,
                assurance,
            })
        }
    }

    fn core_hook() -> AuthCoreHook {
        AuthCoreHook::new(std::sync::Arc::new(TestPrincipalValidator))
            .try_with_session_hint_subprotocol_prefix("bridgefu.attach.")
            .unwrap()
    }

    #[test]
    fn extract_bearer_handles_canonical_form() {
        assert_eq!(extract_bearer(Some("Bearer abc123")), Some("abc123"));
        assert_eq!(extract_bearer(Some("bearer xyz")), Some("xyz"));
    }

    #[test]
    fn extract_bearer_rejects_other_schemes() {
        assert_eq!(extract_bearer(Some("Basic dXNlcjpwYXNz")), None);
        assert_eq!(extract_bearer(Some("")), None);
        assert_eq!(extract_bearer(None), None);
    }

    #[tokio::test]
    async fn anonymous_hook_accepts_all() {
        let hook = AnonymousAuth;
        let ctx = WhipAuthHook::authenticate(
            &hook,
            "POST",
            "/whip/x",
            None,
            "127.0.0.1:1".parse().unwrap(),
        )
        .await
        .expect("anonymous should accept");
        assert_eq!(ctx.subject, "anonymous");
    }

    #[tokio::test]
    async fn bearer_hook_rejects_missing_token() {
        let hook = BearerStaticTokenAuth::new("secret");
        let res = WhipAuthHook::authenticate(
            &hook,
            "POST",
            "/whip/x",
            None,
            "127.0.0.1:1".parse().unwrap(),
        )
        .await;
        assert!(matches!(res, Err(AuthRejection::Unauthorized { .. })));
    }

    #[tokio::test]
    async fn bearer_hook_accepts_valid_token() {
        let hook = BearerStaticTokenAuth::new("secret");
        let ctx = WhipAuthHook::authenticate(
            &hook,
            "POST",
            "/whip/x",
            Some("secret"),
            "127.0.0.1:1".parse().unwrap(),
        )
        .await
        .expect("valid token should accept");
        assert!(ctx.has_scope("whip:publish"));
    }

    #[test]
    fn auth_context_debug_redacts_identity_and_session_hint() {
        let context = AuthContext {
            subject: "private-subject".into(),
            scopes: vec!["private-scope".into()],
            session_hint: Some("private-attachment-token".into()),
            principal: None,
        };

        let rendered = format!("{context:?}");
        assert!(rendered.contains("has_session_hint: true"));
        assert!(!rendered.contains("private-subject"));
        assert!(!rendered.contains("private-scope"));
        assert!(!rendered.contains("private-attachment-token"));
    }

    #[test]
    fn session_hint_prefix_configuration_is_bounded_and_unambiguous() {
        for prefix in [
            "",
            "tok",
            "token.",
            "token.private",
            "rvoip",
            "rvoip.webrtc.v1",
            "rvoip.webrtc.v1.private",
            "bad prefix",
            "bad/",
        ] {
            assert!(matches!(
                AuthCoreHook::new(std::sync::Arc::new(TestPrincipalValidator))
                    .try_with_session_hint_subprotocol_prefix(prefix),
                Err(SessionHintConfigError::InvalidPrefix)
            ));
        }
        assert!(matches!(
            AuthCoreHook::new(std::sync::Arc::new(TestPrincipalValidator))
                .try_with_session_hint_subprotocol_prefix(
                    "x".repeat(MAX_SESSION_HINT_PREFIX_BYTES + 1)
                ),
            Err(SessionHintConfigError::InvalidPrefix)
        ));
    }

    #[tokio::test]
    async fn auth_core_websocket_keeps_authentication_and_attachment_separate() {
        let hook = core_hook();
        assert!(hook.subprotocol_is_private("token.valid-auth"));
        assert!(hook.subprotocol_is_private("bridgefu.attach.private-attachment-token"));
        assert!(!hook.subprotocol_is_private("rvoip.webrtc.v1"));
        let context = WsAuthHook::authenticate(
            &hook,
            &[
                "rvoip.webrtc.v1".into(),
                "token.valid-auth".into(),
                "bridgefu.attach.private-attachment-token".into(),
            ],
            None,
            "127.0.0.1:1".parse().unwrap(),
        )
        .await
        .unwrap();

        assert_eq!(
            context.session_hint.as_deref(),
            Some("private-attachment-token")
        );
        let principal = context.principal.unwrap();
        assert_eq!(principal.tenant.as_deref(), Some("private-tenant"));
    }

    #[tokio::test]
    async fn auth_core_websocket_rejects_missing_duplicate_or_oversized_hints() {
        let peer = "127.0.0.1:1".parse().unwrap();
        for subprotocols in [
            vec!["token.valid-auth".into()],
            vec![
                "token.valid-auth".into(),
                "bridgefu.attach.one".into(),
                "bridgefu.attach.two".into(),
            ],
            vec![
                "token.valid-auth".into(),
                format!("bridgefu.attach.{}", "x".repeat(MAX_SESSION_HINT_BYTES + 1)),
            ],
        ] {
            assert!(matches!(
                WsAuthHook::authenticate(&core_hook(), &subprotocols, None, peer).await,
                Err(AuthRejection::Forbidden)
            ));
        }
    }

    #[tokio::test]
    async fn auth_core_websocket_rejects_ambiguous_bearer_sources() {
        let duplicate_subprotocols = vec![
            "token.valid-auth".into(),
            "token.valid-auth".into(),
            "bridgefu.attach.hint".into(),
        ];
        assert!(matches!(
            WsAuthHook::authenticate(
                &core_hook(),
                &duplicate_subprotocols,
                None,
                "127.0.0.1:1".parse().unwrap(),
            )
            .await,
            Err(AuthRejection::Unauthorized { .. })
        ));

        let query_and_subprotocol = AuthCoreHook::new(std::sync::Arc::new(TestPrincipalValidator))
            .allow_query_tokens(true)
            .try_with_session_hint_subprotocol_prefix("bridgefu.attach.")
            .unwrap();
        assert!(matches!(
            WsAuthHook::authenticate(
                &query_and_subprotocol,
                &["token.valid-auth".into(), "bridgefu.attach.hint".into(),],
                Some("valid-auth"),
                "127.0.0.1:1".parse().unwrap(),
            )
            .await,
            Err(AuthRejection::Unauthorized { .. })
        ));
    }
}
