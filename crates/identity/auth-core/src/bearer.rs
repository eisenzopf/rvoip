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
use rvoip_core_traits::identity::{IdentityAssurance, Jwk};

// Compatibility re-export: the canonical type now lives in the dependency-
// cycle-free core trait surface, but existing `auth_core::bearer::*` imports
// continue to work.
pub use rvoip_core_traits::identity::{
    AuthenticatedPrincipal, AuthenticationMethod, BearerAuthError, PrincipalOwnershipKey,
};

/// Reject a validator result whose credential has already expired.
///
/// JWT libraries commonly allow clock-skew leeway while decoding.  The
/// principal boundary is deliberately stricter: once the advertised expiry
/// has passed, protocol adapters must not retain or authorize the principal.
pub fn ensure_principal_active(
    principal: AuthenticatedPrincipal,
) -> Result<AuthenticatedPrincipal, BearerAuthError> {
    if principal.subject.trim().is_empty() {
        return Err(BearerAuthError::Invalid(
            "authenticated principal subject is empty".into(),
        ));
    }
    if principal.subject.chars().any(char::is_control) {
        return Err(BearerAuthError::Invalid(
            "authenticated principal subject contains control characters".into(),
        ));
    }
    for (name, value) in [
        ("issuer", principal.issuer.as_deref()),
        ("tenant", principal.tenant.as_deref()),
    ] {
        if value.is_some_and(|value| value.chars().any(char::is_control)) {
            return Err(BearerAuthError::Invalid(format!(
                "authenticated principal {name} contains control characters"
            )));
        }
    }
    if principal.is_expired() {
        Err(BearerAuthError::Invalid(
            "authenticated principal is expired".into(),
        ))
    } else {
        Ok(principal)
    }
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
        let assurance = self.validate(token).await?;
        ensure_principal_active(AuthenticatedPrincipal::from_assurance(assurance))
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
