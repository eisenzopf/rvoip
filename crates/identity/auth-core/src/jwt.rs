//! JWT-based [`BearerValidator`] for production deployments.
//!
//! Validates RFC 7519 JWTs against a pre-configured signing key
//! (symmetric HMAC, or asymmetric RSA / EC PEM), checks `exp`, and
//! optionally enforces `iss` / `aud` constraints. On success, maps the
//! token's `sub` claim onto a `rvoip_core::identity::IdentityAssurance::UserAuthorized`
//! with whatever `scope` / `scopes` claim the token carried.
//!
//! This is one of the production Bearer validator options in auth-core.
//! [`crate::bearer::bearer_stub`] remains available for local tests, but it
//! accepts any non-empty token and must not be used as a production gate.
//! JWKS/OIDC, OAuth2 introspection, AAuth, DPoP, and RFC 9421 signed-request
//! primitives are provided as separate validators/building blocks.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use jsonwebtoken::{decode, Algorithm, DecodingKey, Validation};
use rvoip_core_traits::identity::IdentityAssurance;
use rvoip_core_traits::ids::IdentityId;
use serde::Deserialize;

use crate::bearer::{
    AuthenticatedPrincipal, AuthenticationMethod, BearerAuthError, BearerValidator,
};
use crate::providers::{
    CredentialAuthError, TokenRevocationChecker, TokenRevocationContext, TokenRevocationStatus,
};

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
    iss: Option<String>,
    #[serde(default)]
    iat: Option<u64>,
    #[serde(default)]
    exp: Option<u64>,
    #[serde(default)]
    jti: Option<String>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    scopes: Option<Vec<String>>,
    #[serde(default)]
    roles: Option<Vec<String>>,
    #[serde(default)]
    realm_access: Option<RoleAccess>,
    #[serde(default)]
    resource_access: Option<HashMap<String, RoleAccess>>,
    #[serde(default, alias = "tenant")]
    tenant_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RoleAccess {
    #[serde(default)]
    roles: Vec<String>,
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
    revocation_checker: Option<Arc<dyn TokenRevocationChecker>>,
}

impl JwtValidator {
    /// Build from an already-constructed `jsonwebtoken` decoding key.
    ///
    /// This is useful for in-process integrations with an auth service that
    /// owns token issuance, such as `rvoip-users-core`.
    pub fn from_decoding_key(decoding_key: DecodingKey, algorithm: Algorithm) -> Self {
        let mut validation = Validation::new(algorithm);
        validation.validate_aud = false;
        Self {
            decoding_key,
            validation,
            revocation_checker: None,
        }
    }

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
            revocation_checker: None,
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
            revocation_checker: None,
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
            revocation_checker: None,
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

    /// Reject JWTs whose `jti` appears in a revocation store.
    ///
    /// When configured, tokens without a `jti` claim are rejected because they
    /// cannot participate in revocation checks.
    pub fn with_revocation_checker(mut self, checker: Arc<dyn TokenRevocationChecker>) -> Self {
        self.revocation_checker = Some(checker);
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
        self.validation
            .set_audience(&auds.into_iter().collect::<Vec<_>>());
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
        Ok(self.validate_principal(token).await?.assurance)
    }

    async fn validate_principal(
        &self,
        token: &str,
    ) -> Result<AuthenticatedPrincipal, BearerAuthError> {
        if token.is_empty() {
            return Err(BearerAuthError::Empty);
        }
        let data = decode::<Claims>(token, &self.decoding_key, &self.validation)
            .map_err(|e| BearerAuthError::Invalid(e.to_string()))?;
        let claims = data.claims;
        let revocation_context = revocation_context_from_claims(&claims);
        check_revocation(
            self.revocation_checker.as_ref(),
            revocation_context.as_ref(),
        )
        .await?;
        let subject = claims.sub.clone();
        let identity = IdentityId::from_string(subject.clone());
        let scopes = scopes_from_claims(
            claims.scope,
            claims.scopes,
            claims.roles,
            claims.realm_access,
            claims.resource_access,
        );
        let assurance = IdentityAssurance::UserAuthorized {
            identity: identity.clone(),
            user_id: identity,
            scopes: scopes.clone(),
        };
        Ok(AuthenticatedPrincipal {
            subject,
            tenant: claims.tenant_id,
            scopes,
            issuer: claims.iss,
            expires_at: claims
                .exp
                .and_then(|seconds| chrono::DateTime::from_timestamp(seconds as i64, 0)),
            method: AuthenticationMethod::Jwt,
            assurance,
        })
    }
}

async fn check_revocation(
    checker: Option<&Arc<dyn TokenRevocationChecker>>,
    context: Option<&TokenRevocationContext>,
) -> Result<(), BearerAuthError> {
    let Some(checker) = checker else {
        return Ok(());
    };
    let Some(context) = context else {
        return Err(BearerAuthError::Invalid(
            "token missing jti for revocation check".into(),
        ));
    };
    match checker.check_token(context).await {
        Ok(TokenRevocationStatus::Active) => Ok(()),
        Ok(TokenRevocationStatus::Revoked) => Err(BearerAuthError::Invalid("token revoked".into())),
        Err(CredentialAuthError::Invalid) | Err(CredentialAuthError::PolicyRejected(_)) => Err(
            BearerAuthError::Invalid("revocation check rejected token".into()),
        ),
        Err(CredentialAuthError::Unavailable(err)) => Err(BearerAuthError::Unavailable(err)),
    }
}

fn revocation_context_from_claims(claims: &Claims) -> Option<TokenRevocationContext> {
    let token_id = claims.jti.clone()?;
    let mut context = TokenRevocationContext::new(token_id).with_subject(claims.sub.clone());
    if let Some(issuer) = claims.iss.clone() {
        context = context.with_issuer(issuer);
    }
    context = context.with_times(
        claims.iat.and_then(unix_seconds_to_system_time),
        claims.exp.and_then(unix_seconds_to_system_time),
    );
    Some(context)
}

fn unix_seconds_to_system_time(seconds: u64) -> Option<SystemTime> {
    UNIX_EPOCH.checked_add(Duration::from_secs(seconds))
}

fn scopes_from_claims(
    scope: Option<String>,
    scopes: Option<Vec<String>>,
    roles: Option<Vec<String>>,
    realm_access: Option<RoleAccess>,
    resource_access: Option<HashMap<String, RoleAccess>>,
) -> Vec<String> {
    let mut values = Vec::new();
    if let Some(scope) = scope {
        values.extend(scope.split_whitespace().map(str::to_string));
    }
    if let Some(scopes) = scopes {
        for scope in scopes {
            push_unique(&mut values, scope);
        }
    }
    if let Some(roles) = roles {
        for role in roles {
            push_unique(&mut values, format!("role:{role}"));
        }
    }
    if let Some(realm_access) = realm_access {
        for role in realm_access.roles {
            push_unique(&mut values, format!("realm:{role}"));
        }
    }
    if let Some(resource_access) = resource_access {
        for (client, access) in resource_access {
            for role in access.roles {
                push_unique(&mut values, format!("{client}:{role}"));
            }
        }
    }
    values
}

fn push_unique(values: &mut Vec<String>, value: String) {
    if !values.contains(&value) {
        values.push(value);
    }
}
