//! Bearer-token validation surface.
//!
//! The [`BearerValidator`] trait is implemented by JWT, JWKS/OIDC, AAuth, and
//! test/demo validators. [`bearer_stub`] remains available for local tests, but
//! production deployments should use a real validator such as
//! [`crate::JwtValidator`] or [`crate::JwksJwtValidator`].
//!
//! Named `BearerAuthError` (not `AuthError`) to avoid colliding with
//! the crate's existing SIP Digest [`crate::AuthError`] type.

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Utc};
use rvoip_core_traits::identity::{IdentityAssurance, Jwk};
use thiserror::Error;

/// Authentication mechanism that established an [`AuthenticatedPrincipal`].
///
/// This deliberately describes the credential family rather than a concrete
/// issuer implementation. Protocol adapters can make authorization decisions
/// without depending on JWT, OIDC, SIP, or AAuth implementation crates.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum AuthenticationMethod {
    Anonymous,
    Bearer,
    Jwt,
    Oidc,
    OAuth2Introspection,
    Dpop,
    SipDigest,
    MutualTls,
    AAuth,
    ApiKey,
}

/// Transport-neutral result of a successful authentication.
///
/// Older rvoip authentication hooks returned only [`IdentityAssurance`],
/// which meant adapters lost the token subject, tenant, issuer, expiry, and
/// scopes before they could enforce resource ownership. This structure is the
/// common identity carried from the signaling boundary into SIP, WebRTC,
/// UCTP, and application policy.
#[derive(Clone, Debug)]
pub struct AuthenticatedPrincipal {
    pub subject: String,
    pub tenant: Option<String>,
    pub scopes: Vec<String>,
    pub issuer: Option<String>,
    pub expires_at: Option<DateTime<Utc>>,
    pub method: AuthenticationMethod,
    pub assurance: IdentityAssurance,
}

impl AuthenticatedPrincipal {
    pub fn anonymous() -> Self {
        Self {
            subject: "anonymous".into(),
            tenant: None,
            scopes: Vec::new(),
            issuer: None,
            expires_at: None,
            method: AuthenticationMethod::Anonymous,
            assurance: IdentityAssurance::Anonymous,
        }
    }

    pub fn has_scope(&self, scope: &str) -> bool {
        self.scopes.iter().any(|candidate| candidate == scope)
    }

    pub fn require_scope(&self, scope: &str) -> Result<(), BearerAuthError> {
        if self.has_scope(scope) {
            Ok(())
        } else {
            Err(BearerAuthError::Invalid(format!(
                "principal is missing required scope {scope}"
            )))
        }
    }

    /// Compatibility mapping for validators that have not yet overridden
    /// [`BearerValidator::validate_principal`]. Production JWT/JWKS/OAuth
    /// validators should return a richer principal directly.
    pub fn from_assurance(assurance: IdentityAssurance) -> Self {
        let (subject, scopes, expires_at) = match &assurance {
            IdentityAssurance::Anonymous => ("anonymous".into(), Vec::new(), None),
            IdentityAssurance::Pseudonymous { ephemeral_key } => {
                let subject = ephemeral_key
                    .0
                    .get("kid")
                    .and_then(serde_json::Value::as_str)
                    .unwrap_or("pseudonymous")
                    .to_string();
                (subject, Vec::new(), None)
            }
            IdentityAssurance::Identified { credential_kind } => {
                (format!("identified:{credential_kind:?}"), Vec::new(), None)
            }
            IdentityAssurance::TaskScoped {
                identity,
                scopes,
                expires_at,
                ..
            } => (identity.to_string(), scopes.clone(), Some(*expires_at)),
            IdentityAssurance::UserAuthorized {
                identity, scopes, ..
            } => (identity.to_string(), scopes.clone(), None),
            IdentityAssurance::DtlsFingerprint { value, .. } => {
                (format!("dtls:{value}"), Vec::new(), None)
            }
        };
        Self {
            subject,
            tenant: None,
            scopes,
            issuer: None,
            expires_at,
            method: AuthenticationMethod::Bearer,
            assurance,
        }
    }
}

#[derive(Debug, Error)]
pub enum BearerAuthError {
    #[error("empty bearer token")]
    Empty,

    #[error("invalid bearer token: {0}")]
    Invalid(String),

    #[error("validator unavailable: {0}")]
    Unavailable(String),
}

/// Validates a bearer token and produces the resulting [`IdentityAssurance`]
/// for the authenticated peer.
#[async_trait]
pub trait BearerValidator: Send + Sync {
    async fn validate(&self, token: &str) -> Result<IdentityAssurance, BearerAuthError>;

    /// Validate a token while retaining the authorization attributes needed
    /// by protocol adapters. Existing third-party validators remain source
    /// compatible through this default mapping.
    async fn validate_principal(
        &self,
        token: &str,
    ) -> Result<AuthenticatedPrincipal, BearerAuthError> {
        self.validate(token)
            .await
            .map(AuthenticatedPrincipal::from_assurance)
    }
}

/// Build the v0 stub validator.
///
/// Behavior: any non-empty token is accepted; the returned
/// `IdentityAssurance::Pseudonymous { ephemeral_key }` carries a freshly
/// generated throwaway JWK (the rvoip-core `Jwk` type is intentionally
/// opaque in v0). Empty tokens are rejected with [`BearerAuthError::Empty`].
pub fn bearer_stub() -> Arc<dyn BearerValidator> {
    Arc::new(StubBearerValidator)
}

struct StubBearerValidator;

#[async_trait]
impl BearerValidator for StubBearerValidator {
    async fn validate(&self, token: &str) -> Result<IdentityAssurance, BearerAuthError> {
        if token.is_empty() {
            return Err(BearerAuthError::Empty);
        }
        let ephemeral_key = Jwk(serde_json::json!({
            "kty": "stub",
            "kid": uuid::Uuid::new_v4().simple().to_string(),
        }));
        Ok(IdentityAssurance::Pseudonymous { ephemeral_key })
    }
}
