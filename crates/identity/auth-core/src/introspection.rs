//! OAuth2 token introspection Bearer validator.
//!
//! This validator implements the client side of RFC 7662-style token
//! introspection for opaque Bearer tokens. It is useful when an IdP does not
//! expose JWTs/JWKS to protocol services, or when immediate revocation must be
//! enforced by asking the authorization server on each validation.

use std::collections::HashSet;
use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use chrono::Utc;
use rvoip_core_traits::identity::IdentityAssurance;
use rvoip_core_traits::ids::IdentityId;
use serde::Deserialize;
use sha2::{Digest, Sha256};
use url::Url;

use crate::bearer::{
    unix_time_from_seconds, validate_optional_token_id, AuthenticatedPrincipal,
    AuthenticationMethod, BearerAuthError, BearerValidator, ValidatedBearer,
};

/// OAuth2 token introspection validator for opaque Bearer tokens.
#[derive(Clone)]
pub struct OAuth2IntrospectionValidator {
    inner: Arc<Inner>,
}

#[derive(Clone)]
struct Inner {
    endpoint: Url,
    client: reqwest::Client,
    client_auth: Option<IntrospectionClientAuth>,
    issuers: Option<HashSet<String>>,
    audiences: Option<HashSet<String>>,
    require_token_id: bool,
}

#[derive(Clone)]
enum IntrospectionClientAuth {
    Basic {
        client_id: String,
        client_secret: String,
    },
    Bearer(String),
}

#[derive(Debug, Deserialize)]
struct IntrospectionResponse {
    active: bool,
    #[serde(default)]
    sub: Option<String>,
    #[serde(default)]
    username: Option<String>,
    #[serde(default)]
    client_id: Option<String>,
    #[serde(default)]
    scope: Option<String>,
    #[serde(default)]
    scopes: Option<Vec<String>>,
    #[serde(default)]
    iss: Option<String>,
    #[serde(default)]
    exp: Option<u64>,
    #[serde(default)]
    iat: Option<u64>,
    #[serde(default, alias = "token_id")]
    jti: Option<String>,
    #[serde(default, alias = "tenant", alias = "tid")]
    tenant_id: Option<String>,
    #[serde(default)]
    aud: Option<IntrospectionAudience>,
}

#[derive(Debug, Deserialize)]
#[serde(untagged)]
enum IntrospectionAudience {
    One(String),
    Many(Vec<String>),
}

impl OAuth2IntrospectionValidator {
    /// Create an introspection validator for an RFC 7662 endpoint.
    pub fn new(endpoint: Url) -> Self {
        let client = reqwest::Client::builder()
            .user_agent("rvoip-auth-core/0.1 (oauth2-introspection)")
            .timeout(Duration::from_secs(10))
            .build()
            .expect("reqwest::Client::builder default config never fails");
        Self {
            inner: Arc::new(Inner {
                endpoint,
                client,
                client_auth: None,
                issuers: None,
                audiences: None,
                require_token_id: false,
            }),
        }
    }

    /// Authenticate to the introspection endpoint with HTTP Basic client auth.
    pub fn with_basic_client_auth(
        self,
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
    ) -> Self {
        let inner = &*self.inner;
        Self {
            inner: Arc::new(Inner {
                endpoint: inner.endpoint.clone(),
                client: inner.client.clone(),
                client_auth: Some(IntrospectionClientAuth::Basic {
                    client_id: client_id.into(),
                    client_secret: client_secret.into(),
                }),
                issuers: inner.issuers.clone(),
                audiences: inner.audiences.clone(),
                require_token_id: inner.require_token_id,
            }),
        }
    }

    /// Authenticate to the introspection endpoint with a Bearer token.
    pub fn with_bearer_client_auth(self, token: impl Into<String>) -> Self {
        let inner = &*self.inner;
        Self {
            inner: Arc::new(Inner {
                endpoint: inner.endpoint.clone(),
                client: inner.client.clone(),
                client_auth: Some(IntrospectionClientAuth::Bearer(token.into())),
                issuers: inner.issuers.clone(),
                audiences: inner.audiences.clone(),
                require_token_id: inner.require_token_id,
            }),
        }
    }

    /// Require the introspection response `iss` to match one configured issuer.
    pub fn with_issuer<I, S>(self, issuers: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let inner = &*self.inner;
        Self {
            inner: Arc::new(Inner {
                endpoint: inner.endpoint.clone(),
                client: inner.client.clone(),
                client_auth: inner.client_auth.clone(),
                issuers: Some(
                    issuers
                        .into_iter()
                        .map(|issuer| issuer.as_ref().to_string())
                        .collect(),
                ),
                audiences: inner.audiences.clone(),
                require_token_id: inner.require_token_id,
            }),
        }
    }

    /// Require the introspection response `aud` to contain one configured
    /// audience.
    pub fn with_audience<I, S>(self, audiences: I) -> Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<str>,
    {
        let inner = &*self.inner;
        Self {
            inner: Arc::new(Inner {
                endpoint: inner.endpoint.clone(),
                client: inner.client.clone(),
                client_auth: inner.client_auth.clone(),
                issuers: inner.issuers.clone(),
                audiences: Some(
                    audiences
                        .into_iter()
                        .map(|audience| audience.as_ref().to_string())
                        .collect(),
                ),
                require_token_id: inner.require_token_id,
            }),
        }
    }

    /// Require the introspection server to return a non-empty, bounded `jti`
    /// or `token_id` rather than using a local credential fingerprint.
    pub fn with_required_token_id(self) -> Self {
        let inner = &*self.inner;
        Self {
            inner: Arc::new(Inner {
                endpoint: inner.endpoint.clone(),
                client: inner.client.clone(),
                client_auth: inner.client_auth.clone(),
                issuers: inner.issuers.clone(),
                audiences: inner.audiences.clone(),
                require_token_id: true,
            }),
        }
    }

    /// Build into a `Arc<dyn BearerValidator>`.
    pub fn into_arc(self) -> Arc<dyn BearerValidator> {
        Arc::new(self)
    }

    async fn introspect(&self, token: &str) -> Result<IntrospectionResponse, BearerAuthError> {
        let mut request = self
            .inner
            .client
            .post(self.inner.endpoint.clone())
            .form(&[("token", token)]);
        if let Some(client_auth) = &self.inner.client_auth {
            request = match client_auth {
                IntrospectionClientAuth::Basic {
                    client_id,
                    client_secret,
                } => request.basic_auth(client_id, Some(client_secret)),
                IntrospectionClientAuth::Bearer(token) => request.bearer_auth(token),
            };
        }
        let response = request
            .send()
            .await
            .map_err(|err| BearerAuthError::Unavailable(format!("introspection: {err}")))?;
        if !response.status().is_success() {
            return Err(BearerAuthError::Unavailable(format!(
                "introspection endpoint returned {}",
                response.status()
            )));
        }
        response
            .json::<IntrospectionResponse>()
            .await
            .map_err(|err| BearerAuthError::Unavailable(format!("introspection parse: {err}")))
    }
}

#[async_trait]
impl BearerValidator for OAuth2IntrospectionValidator {
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
        let response = self.introspect(token).await?;
        if !response.active {
            return Err(BearerAuthError::Invalid("inactive token".into()));
        }
        validate_issuer(&response, self.inner.issuers.as_ref())?;
        validate_audience(&response, self.inner.audiences.as_ref())?;

        let explicit_token_id = validate_optional_token_id(response.jti.clone())?;
        if self.inner.require_token_id && explicit_token_id.is_none() {
            return Err(BearerAuthError::Invalid(
                "introspection response missing required token id".into(),
            ));
        }
        let token_id = explicit_token_id.or_else(|| Some(credential_fingerprint(token)));
        let issued_at = response
            .iat
            .map(|iat| unix_time_from_seconds(iat, "introspection iat"))
            .transpose()?;

        let expires_at = response.exp.map(expiration_from_unix).transpose()?;
        if expires_at.is_some_and(|expiry| expiry <= Utc::now()) {
            return Err(BearerAuthError::Invalid(
                "active introspection response is expired".into(),
            ));
        }

        let subject = response
            .sub
            .or(response.username)
            .or(response.client_id)
            .ok_or_else(|| {
                BearerAuthError::Invalid("active introspection response missing subject".into())
            })?;
        let identity = IdentityId::from_string(subject.clone());
        let scopes = scopes_from_response(response.scope, response.scopes);
        let assurance = IdentityAssurance::UserAuthorized {
            identity: identity.clone(),
            user_id: identity,
            scopes: scopes.clone(),
        };
        ValidatedBearer::new(
            AuthenticatedPrincipal {
                subject,
                tenant: response.tenant_id,
                scopes,
                issuer: response.iss,
                expires_at,
                method: AuthenticationMethod::OAuth2Introspection,
                assurance,
            },
            token_id,
            issued_at,
        )
    }
}

/// Produce a stable opaque-token identifier without retaining the credential.
/// The result is correlation-sensitive and is only returned through the
/// redacting [`ValidatedBearer`] metadata surface; it is never logged here.
fn credential_fingerprint(token: &str) -> String {
    let digest = Sha256::digest(token.as_bytes());
    format!("sha256:{}", hex::encode(digest))
}

fn expiration_from_unix(seconds: u64) -> Result<chrono::DateTime<Utc>, BearerAuthError> {
    i64::try_from(seconds)
        .ok()
        .and_then(|seconds| chrono::DateTime::from_timestamp(seconds, 0))
        .ok_or_else(|| {
            BearerAuthError::Invalid("introspection exp is outside the supported range".into())
        })
}

fn validate_issuer(
    response: &IntrospectionResponse,
    expected: Option<&HashSet<String>>,
) -> Result<(), BearerAuthError> {
    let Some(expected) = expected else {
        return Ok(());
    };
    let Some(issuer) = response.iss.as_ref() else {
        return Err(BearerAuthError::Invalid(
            "introspection response missing issuer".into(),
        ));
    };
    if expected.contains(issuer) {
        Ok(())
    } else {
        Err(BearerAuthError::Invalid(
            "introspection issuer mismatch".into(),
        ))
    }
}

fn validate_audience(
    response: &IntrospectionResponse,
    expected: Option<&HashSet<String>>,
) -> Result<(), BearerAuthError> {
    let Some(expected) = expected else {
        return Ok(());
    };
    let audiences: Vec<&str> = match response.aud.as_ref() {
        Some(IntrospectionAudience::One(audience)) => vec![audience.as_str()],
        Some(IntrospectionAudience::Many(audiences)) => {
            audiences.iter().map(String::as_str).collect()
        }
        None => Vec::new(),
    };
    if audiences
        .iter()
        .any(|audience| expected.contains(*audience))
    {
        Ok(())
    } else {
        Err(BearerAuthError::Invalid(
            "introspection audience mismatch".into(),
        ))
    }
}

fn scopes_from_response(scope: Option<String>, scopes: Option<Vec<String>>) -> Vec<String> {
    let mut values = Vec::new();
    if let Some(scope) = scope {
        values.extend(scope.split_whitespace().map(str::to_string));
    }
    if let Some(scopes) = scopes {
        for scope in scopes {
            if !values.contains(&scope) {
                values.push(scope);
            }
        }
    }
    values
}
