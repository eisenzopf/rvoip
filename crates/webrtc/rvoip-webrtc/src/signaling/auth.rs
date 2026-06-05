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
//! deployment's policy code; the hook returns an [`AuthContext`] echoed
//! onto the route, or an [`AuthRejection`] mapped to a 401/403/429.

use std::net::SocketAddr;

use async_trait::async_trait;

/// Authentication context attached to a request after a successful
/// authenticate hook. Echoed onto the route for downstream consumers
/// (orchestrator, identity verification, audit log).
#[derive(Clone, Debug)]
pub struct AuthContext {
    /// Opaque tenant / user identifier — meaning is deployment-defined.
    pub subject: String,
    /// Scopes granted (e.g. `["whip:publish", "whep:subscribe"]`).
    pub scopes: Vec<String>,
    /// Optional hint about which session the caller is acting on
    /// (extracted from the URL path or a custom header).
    pub session_hint: Option<String>,
}

impl AuthContext {
    pub fn anonymous() -> Self {
        Self {
            subject: "anonymous".to_string(),
            scopes: Vec::new(),
            session_hint: None,
        }
    }

    pub fn has_scope(&self, scope: &str) -> bool {
        self.scopes.iter().any(|s| s == scope)
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
            }),
            _ => Err(AuthRejection::Unauthorized {
                www_authenticate: format!("Bearer realm=\"{}\"", self.realm),
            }),
        }
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
}
