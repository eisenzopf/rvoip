//! JWKS-fetching [`BearerValidator`] for OIDC-style deployments.
//!
//! Connects the [`crate::jwt::JwtValidator`] surface to real identity
//! providers (Auth0, Okta, Cognito, Keycloak, ...) that publish their
//! signing keys at a `/.well-known/jwks.json` endpoint. Behavior:
//!
//! 1. Parse the incoming JWT's *header* (no signature check yet) to
//!    extract the `kid`.
//! 2. Look up the kid in the local cache. Cache miss → fetch the JWKS
//!    document, parse every key, store by kid with the configured TTL.
//! 3. Validate the full token against the resolved [`DecodingKey`] +
//!    the configured [`Validation`].
//! 4. Map `sub` / `scope` / `scopes` to
//!    `rvoip_core::identity::IdentityAssurance::UserAuthorized`.
//!
//! The cache holds parsed keys, not raw JWKS bytes, so the validate hot
//! path is signature-verify only. TTL defaults to 1 hour — typical
//! issuers rotate keys on the order of days, so 1h cache + on-miss
//! refresh handles rotation without thundering-herd refetches.

use std::collections::{HashMap, HashSet};
use std::sync::Arc;
use std::time::{Duration, SystemTime};

use async_trait::async_trait;
use jsonwebtoken::{decode, decode_header, Algorithm, DecodingKey, Validation};
use moka::future::Cache;
use rvoip_core_traits::identity::IdentityAssurance;
use rvoip_core_traits::ids::IdentityId;
use serde::Deserialize;
use tracing::{debug, warn};
use url::Url;

use crate::bearer::{
    unix_time_from_seconds, validate_optional_token_id, AuthenticatedPrincipal,
    AuthenticationMethod, BearerAuthError, BearerValidator, ValidatedBearer,
};
use crate::providers::{
    CredentialAuthError, TokenRevocationChecker, TokenRevocationContext, TokenRevocationStatus,
};

/// Default JWKS cache TTL. Issuers typically rotate signing keys on
/// the order of days; 1h covers normal operation without burning
/// requests on every validate. Tune via [`JwksJwtValidator::with_cache_ttl`].
pub const DEFAULT_JWKS_CACHE_TTL: Duration = Duration::from_secs(3600);

/// Maximum number of keys cached. JWKS documents usually carry 1-5
/// keys (current + a small set of rotating ones), so 64 is plenty
/// without paying for an unbounded cache.
const JWKS_CACHE_MAX_CAPACITY: u64 = 64;

#[derive(Debug, Deserialize)]
struct JwksDocument {
    keys: Vec<JwksKey>,
}

#[derive(Debug, Deserialize)]
struct JwksKey {
    kty: String,
    kid: Option<String>,
    // RSA fields.
    n: Option<String>,
    e: Option<String>,
    // EC fields.
    #[allow(dead_code)] // surfaced for future per-curve dispatch
    crv: Option<String>,
    x: Option<String>,
    y: Option<String>,
}

#[derive(Debug, Deserialize)]
struct TokenClaims {
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
    #[serde(default, alias = "tenant", alias = "tid")]
    tenant_id: Option<String>,
}

#[derive(Debug, Deserialize)]
struct RoleAccess {
    #[serde(default)]
    roles: Vec<String>,
}

/// Bearer validator that resolves signing keys from a remote JWKS
/// endpoint. See module-level docs for behavior. Cheap to clone (the
/// inner state is an Arc).
#[derive(Clone)]
pub struct JwksJwtValidator {
    inner: Arc<Inner>,
}

struct Inner {
    jwks_url: Url,
    client: reqwest::Client,
    cache: Cache<String, DecodingKey>,
    validation: Validation,
    revocation_checker: Option<Arc<dyn TokenRevocationChecker>>,
    require_jti: bool,
}

impl JwksJwtValidator {
    /// Build a validator against the given JWKS URL. The JWKS isn't
    /// fetched until the first validate call (lazy bootstrap so
    /// construction can't fail on transient network errors). Default
    /// algorithm: RS256 (the dominant OIDC choice). Callers needing
    /// ES256 / EdDSA tokens override via [`Self::with_algorithms`].
    pub fn new(jwks_url: Url) -> Self {
        let mut validation = Validation::new(Algorithm::RS256);
        validation.validate_aud = false;
        Self::new_with_validation(jwks_url, validation)
    }

    /// Variant with an explicit `Validation` config (allows tuning
    /// algorithms, leeway, required claims). Most callers should use
    /// [`Self::new`] + the `with_*` builders.
    pub fn new_with_validation(jwks_url: Url, validation: Validation) -> Self {
        let client = reqwest::Client::builder()
            .user_agent("rvoip-auth-core/0.1 (jwks)")
            .timeout(Duration::from_secs(10))
            .build()
            .expect("reqwest::Client::builder default config never fails");
        Self {
            inner: Arc::new(Inner {
                jwks_url,
                client,
                cache: Cache::builder()
                    .max_capacity(JWKS_CACHE_MAX_CAPACITY)
                    .time_to_live(DEFAULT_JWKS_CACHE_TTL)
                    .build(),
                validation,
                revocation_checker: None,
                require_jti: false,
            }),
        }
    }

    /// Override the JWKS cache TTL. Drops the existing cache contents
    /// (call before tokens start flowing).
    pub fn with_cache_ttl(self, ttl: Duration) -> Self {
        let inner = &*self.inner;
        let new_cache = Cache::builder()
            .max_capacity(JWKS_CACHE_MAX_CAPACITY)
            .time_to_live(ttl)
            .build();
        Self {
            inner: Arc::new(Inner {
                jwks_url: inner.jwks_url.clone(),
                client: inner.client.clone(),
                cache: new_cache,
                validation: inner.validation.clone(),
                revocation_checker: inner.revocation_checker.clone(),
                require_jti: inner.require_jti,
            }),
        }
    }

    /// Require the token's `aud` claim to match one of `audiences`.
    pub fn with_audience<I, S>(self, audiences: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let inner = &*self.inner;
        let mut validation = inner.validation.clone();
        let auds: HashSet<String> = audiences
            .into_iter()
            .map(|s| s.as_ref().to_string())
            .collect();
        validation.set_audience(&auds.into_iter().collect::<Vec<_>>());
        validation.validate_aud = true;
        Self {
            inner: Arc::new(Inner {
                jwks_url: inner.jwks_url.clone(),
                client: inner.client.clone(),
                cache: inner.cache.clone(),
                validation,
                revocation_checker: inner.revocation_checker.clone(),
                require_jti: inner.require_jti,
            }),
        }
    }

    /// Require the token's `iss` claim to match one of `issuers`.
    pub fn with_issuer<I, S>(self, issuers: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let inner = &*self.inner;
        let mut validation = inner.validation.clone();
        validation.set_issuer(
            &issuers
                .into_iter()
                .map(|s| s.as_ref().to_string())
                .collect::<Vec<_>>(),
        );
        Self {
            inner: Arc::new(Inner {
                jwks_url: inner.jwks_url.clone(),
                client: inner.client.clone(),
                cache: inner.cache.clone(),
                validation,
                revocation_checker: inner.revocation_checker.clone(),
                require_jti: inner.require_jti,
            }),
        }
    }

    /// Restrict allowed algorithms (default is RS256 + ES256).
    pub fn with_algorithms(self, algorithms: Vec<Algorithm>) -> Self {
        let inner = &*self.inner;
        let mut validation = inner.validation.clone();
        validation.algorithms = algorithms;
        Self {
            inner: Arc::new(Inner {
                jwks_url: inner.jwks_url.clone(),
                client: inner.client.clone(),
                cache: inner.cache.clone(),
                validation,
                revocation_checker: inner.revocation_checker.clone(),
                require_jti: inner.require_jti,
            }),
        }
    }

    /// Reject JWTs whose `jti` appears in a revocation store.
    ///
    /// When configured, tokens without a `jti` claim are rejected because they
    /// cannot participate in revocation checks.
    pub fn with_revocation_checker(self, checker: Arc<dyn TokenRevocationChecker>) -> Self {
        let inner = &*self.inner;
        Self {
            inner: Arc::new(Inner {
                jwks_url: inner.jwks_url.clone(),
                client: inner.client.clone(),
                cache: inner.cache.clone(),
                validation: inner.validation.clone(),
                revocation_checker: Some(checker),
                require_jti: inner.require_jti,
            }),
        }
    }

    /// Require a non-empty, bounded JWT `jti` even without a revocation store.
    ///
    /// Production deployments that use token IDs for replay, lease, or audit
    /// correlation should enable this policy. Configuring a revocation checker
    /// already requires `jti` regardless of this setting.
    pub fn with_required_jti(self) -> Self {
        let inner = &*self.inner;
        Self {
            inner: Arc::new(Inner {
                jwks_url: inner.jwks_url.clone(),
                client: inner.client.clone(),
                cache: inner.cache.clone(),
                validation: inner.validation.clone(),
                revocation_checker: inner.revocation_checker.clone(),
                require_jti: true,
            }),
        }
    }

    /// Build into a `Arc<dyn BearerValidator>` for adapter config.
    pub fn into_arc(self) -> Arc<dyn BearerValidator> {
        Arc::new(self)
    }

    /// Resolve a signing key for `kid` from the cache, fetching the
    /// JWKS document on cache miss. Keys with unsupported `kty` /
    /// missing components are skipped silently; an `Invalid` error
    /// surfaces only when the kid still isn't found after a fresh
    /// fetch.
    async fn resolve_key(&self, kid: &str) -> Result<DecodingKey, BearerAuthError> {
        if let Some(key) = self.inner.cache.get(kid).await {
            return Ok(key);
        }
        // Cache miss — fetch JWKS, populate every parseable key, then
        // re-check the cache for the kid we want.
        debug!(kid = %kid, "jwks: cache miss, refetching");
        let doc = self.fetch_jwks().await?;
        for jwk in doc.keys {
            let Some(jwk_kid) = jwk.kid.clone() else {
                // No kid on this entry — skip; we can't address it
                // from the token header.
                continue;
            };
            match decoding_key_from_jwk(&jwk) {
                Ok(key) => {
                    self.inner.cache.insert(jwk_kid, key).await;
                }
                Err(e) => {
                    warn!(
                        kid = %jwk_kid,
                        error = %e,
                        "jwks: skipping unparseable key"
                    );
                }
            }
        }
        self.inner
            .cache
            .get(kid)
            .await
            .ok_or_else(|| BearerAuthError::Invalid(format!("no signing key for kid={}", kid)))
    }

    async fn fetch_jwks(&self) -> Result<JwksDocument, BearerAuthError> {
        let resp = self
            .inner
            .client
            .get(self.inner.jwks_url.clone())
            .send()
            .await
            .map_err(|e| BearerAuthError::Unavailable(format!("JWKS fetch: {e}")))?;
        if !resp.status().is_success() {
            return Err(BearerAuthError::Unavailable(format!(
                "JWKS endpoint returned {}",
                resp.status()
            )));
        }
        resp.json::<JwksDocument>()
            .await
            .map_err(|e| BearerAuthError::Unavailable(format!("JWKS parse: {e}")))
    }
}

fn decoding_key_from_jwk(jwk: &JwksKey) -> Result<DecodingKey, BearerAuthError> {
    match jwk.kty.as_str() {
        "RSA" => {
            let n = jwk
                .n
                .as_deref()
                .ok_or_else(|| BearerAuthError::Invalid("RSA jwk missing n".into()))?;
            let e = jwk
                .e
                .as_deref()
                .ok_or_else(|| BearerAuthError::Invalid("RSA jwk missing e".into()))?;
            DecodingKey::from_rsa_components(n, e)
                .map_err(|err| BearerAuthError::Invalid(format!("RSA jwk: {err}")))
        }
        "EC" => {
            let x = jwk
                .x
                .as_deref()
                .ok_or_else(|| BearerAuthError::Invalid("EC jwk missing x".into()))?;
            let y = jwk
                .y
                .as_deref()
                .ok_or_else(|| BearerAuthError::Invalid("EC jwk missing y".into()))?;
            // `from_ec_components` requires `crv` info implicitly via
            // jsonwebtoken's algorithm match. We pass x/y; downstream
            // validation enforces the right algorithm.
            let _ = jwk.crv.as_deref().unwrap_or("P-256");
            DecodingKey::from_ec_components(x, y)
                .map_err(|err| BearerAuthError::Invalid(format!("EC jwk: {err}")))
        }
        "oct" => {
            // RFC 7518 §6.4 oct (symmetric) keys are uncommon in JWKS
            // and call for a separate validator anyway — the JWKS
            // path's whole point is asymmetric verification with
            // public keys distributed via the well-known endpoint.
            // Callers with shared secrets should use
            // `JwtValidator::from_hmac_secret` directly.
            Err(BearerAuthError::Invalid(
                "oct (symmetric) keys in JWKS not supported; use HMAC JwtValidator directly".into(),
            ))
        }
        other => Err(BearerAuthError::Invalid(format!("unsupported kty={other}"))),
    }
}

#[async_trait]
impl BearerValidator for JwksJwtValidator {
    async fn validate(&self, token: &str) -> Result<IdentityAssurance, BearerAuthError> {
        Ok(self.validate_credential(token).await?.principal.assurance)
    }

    async fn validate_principal(
        &self,
        token: &str,
    ) -> Result<AuthenticatedPrincipal, BearerAuthError> {
        Ok(self.validate_credential(token).await?.principal)
    }

    async fn validate_credential(&self, token: &str) -> Result<ValidatedBearer, BearerAuthError> {
        if token.is_empty() {
            return Err(BearerAuthError::Empty);
        }
        // 1. Parse header to extract kid. Tokens without a kid can't
        // be resolved against JWKS; reject them up front.
        let header =
            decode_header(token).map_err(|e| BearerAuthError::Invalid(format!("header: {e}")))?;
        let kid = header
            .kid
            .as_ref()
            .ok_or_else(|| BearerAuthError::Invalid("token header missing kid".into()))?;

        // 2. Resolve signing key (cache lookup or JWKS refetch).
        let key = self.resolve_key(kid).await?;

        // 3. Validate the full token.
        let data = decode::<TokenClaims>(token, &key, &self.inner.validation)
            .map_err(|e| BearerAuthError::Invalid(e.to_string()))?;
        let claims = data.claims;
        let token_id = validate_optional_token_id(claims.jti.clone())?;
        if self.inner.require_jti && token_id.is_none() {
            return Err(BearerAuthError::Invalid(
                "token missing required jti".into(),
            ));
        }
        let issued_at = claims
            .iat
            .map(|iat| unix_time_from_seconds(iat, "iat"))
            .transpose()?;
        let expires_at_system = claims
            .exp
            .map(|exp| unix_time_from_seconds(exp, "exp"))
            .transpose()?;
        let revocation_context = revocation_context_from_claims(
            &claims,
            token_id.as_deref(),
            issued_at,
            expires_at_system,
        );
        check_revocation(
            self.inner.revocation_checker.as_ref(),
            revocation_context.as_ref(),
        )
        .await?;

        // 4. Preserve the authorization claims alongside the legacy
        // IdentityAssurance projection.
        let subject = claims.sub.clone();
        let expires_at = claims.exp.map(expiration_from_unix).transpose()?;
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
        ValidatedBearer::new(
            AuthenticatedPrincipal {
                subject,
                tenant: claims.tenant_id,
                scopes,
                issuer: claims.iss,
                expires_at,
                method: AuthenticationMethod::Oidc,
                assurance,
            },
            token_id,
            issued_at,
        )
    }
}

fn expiration_from_unix(seconds: u64) -> Result<chrono::DateTime<chrono::Utc>, BearerAuthError> {
    i64::try_from(seconds)
        .ok()
        .and_then(|seconds| chrono::DateTime::from_timestamp(seconds, 0))
        .ok_or_else(|| BearerAuthError::Invalid("token exp is outside the supported range".into()))
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

fn revocation_context_from_claims(
    claims: &TokenClaims,
    token_id: Option<&str>,
    issued_at: Option<SystemTime>,
    expires_at: Option<SystemTime>,
) -> Option<TokenRevocationContext> {
    let mut context = TokenRevocationContext::new(token_id?).with_subject(claims.sub.clone());
    if let Some(issuer) = claims.iss.clone() {
        context = context.with_issuer(issuer);
    }
    context = context.with_times(issued_at, expires_at);
    Some(context)
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
