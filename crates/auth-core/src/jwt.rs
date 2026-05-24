//! JWT-based [`BearerValidator`] for production deployments.
//!
//! Validates RFC 7519 JWTs against a pre-configured signing key
//! (symmetric HMAC, or asymmetric RSA / EC PEM), checks `exp`, and
//! optionally enforces `iss` / `aud` constraints. On success, maps the
//! token's `sub` claim onto a [`rvoip_core::identity::IdentityAssurance::UserAuthorized`]
//! with whatever `scope` / `scopes` claim the token carried.
//!
//! This is the first real (non-stub) [`BearerValidator`]; it makes the
//! UCTP coordinator's auth gate (plan A1 / G1) meaningful in
//! production — today's [`crate::bearer::bearer_stub`] accepts any
//! non-empty token, so the gate effectively refuses nobody.
//!
//! DPoP / AAuth / RFC 9421 signed-request validation are separate
//! validators that will land alongside this one.

use std::collections::HashSet;
use std::sync::Arc;

use async_trait::async_trait;
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use rvoip_core::identity::IdentityAssurance;
use rvoip_core::ids::IdentityId;
use serde::Deserialize;

use crate::bearer::{BearerAuthError, BearerValidator};

/// Claims `JwtValidator` decodes from each token. Only `sub` and the
/// scope claim are required for the IdentityAssurance mapping; `iss` /
/// `aud` / `exp` are checked by `jsonwebtoken` against the
/// [`Validation`] config the validator was built with.
///
/// Both `scope` (space-separated string) and `scopes` (array form) are
/// accepted to match the variety of issuer conventions in the wild.
/// Tokens with neither map to an empty scopes Vec.
#[derive(Debug, Deserialize)]
struct Claims {
    sub: String,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    scopes: Option<Vec<String>>,
}

/// Validate JWTs against a single signing key. Constructed from either
/// a symmetric HMAC secret or an asymmetric PEM-encoded public key.
///
/// `validate()` rejects with [`BearerAuthError::Empty`] for an empty
/// token, [`BearerAuthError::Invalid`] for any decode/signature/exp/iss/aud
/// failure (the underlying jsonwebtoken error message is preserved in
/// the variant), and produces
/// [`IdentityAssurance::UserAuthorized`] on success.
pub struct JwtValidator {
    decoding_key: DecodingKey,
    validation: Validation,
}

impl JwtValidator {
    /// HMAC validator — `secret` is the shared HMAC key bytes. Defaults
    /// to HS256; use [`Self::with_algorithm`] to change.
    pub fn from_hmac_secret(secret: &[u8]) -> Self {
        let mut validation = Validation::new(Algorithm::HS256);
        // jsonwebtoken's default Validation has `validate_exp = true`
        // and a 60s leeway; we keep that. Disable aud/iss by default —
        // callers opt in via with_audience / with_issuer.
        validation.set_audience::<&str>(&[]);
        validation.validate_aud = false;
        Self {
            decoding_key: DecodingKey::from_secret(secret),
            validation,
        }
    }

    /// RSA validator from a PEM-encoded public key (`-----BEGIN PUBLIC KEY-----`).
    /// Defaults to RS256.
    pub fn from_rsa_pem(pem: &[u8]) -> Result<Self, BearerAuthError> {
        let key = DecodingKey::from_rsa_pem(pem)
            .map_err(|e| BearerAuthError::Unavailable(format!("invalid RSA PEM: {e}")))?;
        let mut validation = Validation::new(Algorithm::RS256);
        validation.validate_aud = false;
        Ok(Self {
            decoding_key: key,
            validation,
        })
    }

    /// EC validator from a PEM-encoded public key. Defaults to ES256.
    pub fn from_ec_pem(pem: &[u8]) -> Result<Self, BearerAuthError> {
        let key = DecodingKey::from_ec_pem(pem)
            .map_err(|e| BearerAuthError::Unavailable(format!("invalid EC PEM: {e}")))?;
        let mut validation = Validation::new(Algorithm::ES256);
        validation.validate_aud = false;
        Ok(Self {
            decoding_key: key,
            validation,
        })
    }

    /// Override the signing algorithm (e.g. HS384, RS512). Must be
    /// compatible with the key form passed to the constructor — an
    /// HMAC validator with `with_algorithm(Algorithm::RS256)` will
    /// always reject every token.
    pub fn with_algorithm(mut self, algorithm: Algorithm) -> Self {
        self.validation.algorithms = vec![algorithm];
        self
    }

    /// Require the token's `aud` claim to match one of `audiences`.
    /// Tokens without an `aud` (or with a non-matching one) are
    /// rejected as `Invalid`.
    pub fn with_audience<I, S>(mut self, audiences: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let auds: HashSet<String> = audiences
            .into_iter()
            .map(|s| s.as_ref().to_string())
            .collect();
        self.validation.set_audience(&auds.into_iter().collect::<Vec<_>>());
        self.validation.validate_aud = true;
        self
    }

    /// Require the token's `iss` claim to match one of `issuers`.
    pub fn with_issuer<I, S>(mut self, issuers: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        self.validation.set_issuer(
            &issuers
                .into_iter()
                .map(|s| s.as_ref().to_string())
                .collect::<Vec<_>>(),
        );
        self
    }

    /// Build into a `Arc<dyn BearerValidator>` ready for adapter
    /// config. Convenience for the common `UctpQuicConfig::new(...)`
    /// shape that wants an Arc.
    pub fn into_arc(self) -> Arc<dyn BearerValidator> {
        Arc::new(self)
    }
}

#[async_trait]
impl BearerValidator for JwtValidator {
    async fn validate(&self, token: &str) -> Result<IdentityAssurance, BearerAuthError> {
        if token.is_empty() {
            return Err(BearerAuthError::Empty);
        }
        let data = decode::<Claims>(token, &self.decoding_key, &self.validation)
            .map_err(|e| BearerAuthError::Invalid(e.to_string()))?;
        let claims = data.claims;
        let identity = IdentityId::from_string(claims.sub);
        // Merge both scope conventions. RFC 8693 / OAuth 2 commonly
        // uses space-separated `scope`; OIDC tokens often use an
        // explicit `scopes` array. Either is accepted; presence of
        // both means the union.
        let mut scopes: Vec<String> = Vec::new();
        if let Some(scope) = claims.scope {
            scopes.extend(scope.split_whitespace().map(|s| s.to_string()));
        }
        if let Some(list) = claims.scopes {
            for s in list {
                if !scopes.contains(&s) {
                    scopes.push(s);
                }
            }
        }
        Ok(IdentityAssurance::UserAuthorized {
            identity: identity.clone(),
            user_id: identity,
            scopes,
        })
    }
}
