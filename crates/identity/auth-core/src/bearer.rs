//! Bearer-token validation surface.
//!
//! v0 ships a single stub implementation ([`bearer_stub`]) that returns
//! `IdentityAssurance::Pseudonymous` for any non-empty token. Real
//! DPoP / JWT / OIDC / AAuth / RFC 9421 validators land later as
//! additional implementations of [`BearerValidator`].
//!
//! Named `BearerAuthError` (not `AuthError`) to avoid colliding with
//! the crate's existing SIP Digest [`crate::AuthError`] type.

use std::sync::Arc;

use async_trait::async_trait;
use rvoip_core_traits::identity::{IdentityAssurance, Jwk};
use thiserror::Error;

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
