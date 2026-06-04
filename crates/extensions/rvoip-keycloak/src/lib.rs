//! Keycloak/OIDC helpers for RVoIP authentication.
//!
//! This crate is optional. Core protocol crates should depend on
//! `rvoip-auth-core` traits, while applications that use Keycloak can use this
//! extension to build validators and test clients from Keycloak realm settings.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rvoip_auth_core::{
    BearerAuthError, BearerValidator, JwksJwtValidator, OAuth2IntrospectionValidator,
};
use rvoip_core_traits::identity::IdentityAssurance;
use serde::Deserialize;
use thiserror::Error;
use url::Url;

/// Keycloak realm/client configuration.
#[derive(Debug, Clone)]
pub struct KeycloakConfig {
    pub base_url: Url,
    pub realm: String,
    pub client_id: String,
    pub client_secret: Option<String>,
    pub audience: Option<String>,
    pub jwks_cache_ttl: Option<Duration>,
}

impl KeycloakConfig {
    /// Build a Keycloak config from the base server URL and realm.
    pub fn new(base_url: Url, realm: impl Into<String>, client_id: impl Into<String>) -> Self {
        Self {
            base_url,
            realm: realm.into(),
            client_id: client_id.into(),
            client_secret: None,
            audience: None,
            jwks_cache_ttl: None,
        }
    }

    pub fn with_client_secret(mut self, secret: impl Into<String>) -> Self {
        self.client_secret = Some(secret.into());
        self
    }

    pub fn with_audience(mut self, audience: impl Into<String>) -> Self {
        self.audience = Some(audience.into());
        self
    }

    /// Override JWKS cache TTL for validators created from this config.
    pub fn with_jwks_cache_ttl(mut self, ttl: Duration) -> Self {
        self.jwks_cache_ttl = Some(ttl);
        self
    }

    pub fn token_url(&self) -> Result<Url, KeycloakError> {
        self.realm_url("protocol/openid-connect/token")
    }

    pub fn jwks_url(&self) -> Result<Url, KeycloakError> {
        self.realm_url("protocol/openid-connect/certs")
    }

    pub fn discovery_url(&self) -> Result<Url, KeycloakError> {
        self.realm_url(".well-known/openid-configuration")
    }

    pub fn issuer_url(&self) -> Result<Url, KeycloakError> {
        self.realm_url("")
    }

    pub async fn discover(&self) -> Result<KeycloakOidcProvider, KeycloakError> {
        KeycloakOidcProvider::discover(self.clone()).await
    }

    fn realm_url(&self, suffix: &str) -> Result<Url, KeycloakError> {
        let mut url = self.base_url.clone();
        let base_path = url.path().trim_end_matches('/');
        let suffix = suffix.trim_start_matches('/');
        let path = if suffix.is_empty() {
            format!("{base_path}/realms/{}", self.realm)
        } else {
            format!("{base_path}/realms/{}/{suffix}", self.realm)
        };
        url.set_path(&path);
        Ok(url)
    }
}

/// OIDC discovery metadata used by RVoIP validators.
#[derive(Debug, Clone, Deserialize)]
pub struct OidcProviderMetadata {
    pub issuer: String,
    pub jwks_uri: Url,
    pub token_endpoint: Url,
    #[serde(default)]
    pub authorization_endpoint: Option<Url>,
    #[serde(default)]
    pub introspection_endpoint: Option<Url>,
    #[serde(default)]
    pub revocation_endpoint: Option<Url>,
}

/// Discovered Keycloak/OIDC provider configuration.
#[derive(Debug, Clone)]
pub struct KeycloakOidcProvider {
    config: KeycloakConfig,
    metadata: OidcProviderMetadata,
    client: reqwest::Client,
}

impl KeycloakOidcProvider {
    /// Fetch `.well-known/openid-configuration` for a Keycloak realm.
    pub async fn discover(config: KeycloakConfig) -> Result<Self, KeycloakError> {
        let client = reqwest::Client::new();
        let metadata = client
            .get(config.discovery_url()?)
            .send()
            .await?
            .error_for_status()?
            .json::<OidcProviderMetadata>()
            .await?;

        Ok(Self {
            config,
            metadata,
            client,
        })
    }

    /// Return the discovered provider metadata.
    pub fn metadata(&self) -> &OidcProviderMetadata {
        &self.metadata
    }

    /// Build a Bearer validator from discovered issuer and JWKS metadata.
    pub fn bearer_validator(&self) -> Result<KeycloakBearerValidator, KeycloakError> {
        KeycloakBearerValidator::from_metadata(&self.config, &self.metadata)
    }

    /// Build an OAuth2 introspection Bearer validator from discovered metadata.
    ///
    /// Keycloak advertises `introspection_endpoint` for confidential clients.
    /// When a client secret is configured, HTTP Basic client authentication is
    /// applied. The validator enforces the discovered issuer and configured
    /// audience when present.
    pub fn introspection_validator(&self) -> Result<OAuth2IntrospectionValidator, KeycloakError> {
        let endpoint = self
            .metadata
            .introspection_endpoint
            .clone()
            .ok_or_else(|| {
                KeycloakError::Config(
                    "OIDC provider metadata does not include introspection_endpoint".to_string(),
                )
            })?;
        let mut validator = OAuth2IntrospectionValidator::new(endpoint)
            .with_issuer([self.metadata.issuer.as_str()]);
        if let Some(audience) = self.config.audience.as_ref() {
            validator = validator.with_audience([audience.as_str()]);
        }
        if let Some(secret) = self.config.client_secret.as_ref() {
            validator =
                validator.with_basic_client_auth(self.config.client_id.clone(), secret.clone());
        }
        Ok(validator)
    }

    /// Check that the discovered JWKS endpoint is reachable.
    pub async fn health_check(&self) -> Result<KeycloakHealth, KeycloakError> {
        self.client
            .get(self.metadata.jwks_uri.clone())
            .send()
            .await?
            .error_for_status()?;
        Ok(KeycloakHealth {
            issuer: self.metadata.issuer.clone(),
            jwks_uri: self.metadata.jwks_uri.clone(),
            jwks_reachable: true,
            introspection_endpoint: self.metadata.introspection_endpoint.clone(),
            revocation_endpoint: self.metadata.revocation_endpoint.clone(),
            audience: self.config.audience.clone(),
        })
    }
}

/// Result from a lightweight Keycloak provider health check.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeycloakHealth {
    pub issuer: String,
    pub jwks_uri: Url,
    pub jwks_reachable: bool,
    pub introspection_endpoint: Option<Url>,
    pub revocation_endpoint: Option<Url>,
    pub audience: Option<String>,
}

/// Keycloak-backed Bearer validator using the realm JWKS endpoint.
pub struct KeycloakBearerValidator {
    inner: JwksJwtValidator,
}

impl KeycloakBearerValidator {
    pub fn new(config: &KeycloakConfig) -> Result<Self, KeycloakError> {
        let issuer = config.issuer_url()?.to_string();
        let mut validator = JwksJwtValidator::new(config.jwks_url()?).with_issuer([issuer]);
        if let Some(ttl) = config.jwks_cache_ttl {
            validator = validator.with_cache_ttl(ttl);
        }
        if let Some(audience) = config.audience.as_ref() {
            validator = validator.with_audience([audience.as_str()]);
        }
        Ok(Self { inner: validator })
    }

    pub fn from_metadata(
        config: &KeycloakConfig,
        metadata: &OidcProviderMetadata,
    ) -> Result<Self, KeycloakError> {
        let mut validator = JwksJwtValidator::new(metadata.jwks_uri.clone())
            .with_issuer([metadata.issuer.as_str()]);
        if let Some(ttl) = config.jwks_cache_ttl {
            validator = validator.with_cache_ttl(ttl);
        }
        if let Some(audience) = config.audience.as_ref() {
            validator = validator.with_audience([audience.as_str()]);
        }
        Ok(Self { inner: validator })
    }

    pub fn into_arc(self) -> Arc<dyn BearerValidator> {
        Arc::new(self)
    }
}

#[async_trait]
impl BearerValidator for KeycloakBearerValidator {
    async fn validate(&self, token: &str) -> Result<IdentityAssurance, BearerAuthError> {
        self.inner.validate(token).await
    }
}

/// Small password-grant client for local integration fixtures.
///
/// This is meant for tests and demos. Production applications should use their
/// normal OAuth/OIDC login flow rather than collecting user passwords in SIP
/// services.
#[derive(Clone)]
pub struct KeycloakPasswordGrantClient {
    config: KeycloakConfig,
    client: reqwest::Client,
}

impl KeycloakPasswordGrantClient {
    pub fn new(config: KeycloakConfig) -> Self {
        Self {
            config,
            client: reqwest::Client::new(),
        }
    }

    pub async fn access_token(
        &self,
        username: &str,
        password: &str,
    ) -> Result<String, KeycloakError> {
        let mut form = vec![
            ("grant_type", "password".to_string()),
            ("client_id", self.config.client_id.clone()),
            ("username", username.to_string()),
            ("password", password.to_string()),
        ];
        if let Some(secret) = self.config.client_secret.as_ref() {
            form.push(("client_secret", secret.clone()));
        }
        let token = self
            .client
            .post(self.config.token_url()?)
            .form(&form)
            .send()
            .await?
            .error_for_status()?
            .json::<TokenResponse>()
            .await?;
        Ok(token.access_token)
    }

    pub async fn access_token_with_retry(
        &self,
        username: &str,
        password: &str,
        attempts: usize,
        delay: Duration,
    ) -> Result<String, KeycloakError> {
        let mut last_error = None;
        for _ in 0..attempts {
            match self.access_token(username, password).await {
                Ok(token) => return Ok(token),
                Err(err) => last_error = Some(err),
            }
            tokio_sleep(delay).await;
        }
        Err(last_error.unwrap_or(KeycloakError::Unavailable(
            "no token attempts were made".to_string(),
        )))
    }
}

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
}

#[derive(Debug, Error)]
pub enum KeycloakError {
    #[error("invalid Keycloak URL: {0}")]
    Url(#[from] url::ParseError),

    #[error("Keycloak HTTP error: {0}")]
    Http(#[from] reqwest::Error),

    #[error("Keycloak configuration error: {0}")]
    Config(String),

    #[error("Keycloak unavailable: {0}")]
    Unavailable(String),
}

async fn tokio_sleep(delay: Duration) {
    tokio::time::sleep(delay).await;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn keycloak_config_builds_realm_urls() {
        let config = KeycloakConfig::new(
            Url::parse("http://127.0.0.1:18080").unwrap(),
            "rvoip",
            "rvoip-sip",
        )
        .with_client_secret("secret")
        .with_audience("rvoip-sip")
        .with_jwks_cache_ttl(Duration::from_secs(30));

        assert_eq!(config.jwks_cache_ttl, Some(Duration::from_secs(30)));

        assert_eq!(
            config.token_url().unwrap().as_str(),
            "http://127.0.0.1:18080/realms/rvoip/protocol/openid-connect/token"
        );
        assert_eq!(
            config.jwks_url().unwrap().as_str(),
            "http://127.0.0.1:18080/realms/rvoip/protocol/openid-connect/certs"
        );
        assert_eq!(
            config.discovery_url().unwrap().as_str(),
            "http://127.0.0.1:18080/realms/rvoip/.well-known/openid-configuration"
        );
        assert_eq!(
            config.issuer_url().unwrap().as_str(),
            "http://127.0.0.1:18080/realms/rvoip"
        );
    }

    fn test_provider(introspection: Option<Url>) -> KeycloakOidcProvider {
        KeycloakOidcProvider {
            config: KeycloakConfig::new(
                Url::parse("http://127.0.0.1:18080").unwrap(),
                "rvoip",
                "rvoip-sip",
            )
            .with_client_secret("secret")
            .with_audience("rvoip-sip"),
            metadata: OidcProviderMetadata {
                issuer: "http://127.0.0.1:18080/realms/rvoip".to_string(),
                jwks_uri: Url::parse(
                    "http://127.0.0.1:18080/realms/rvoip/protocol/openid-connect/certs",
                )
                .unwrap(),
                token_endpoint: Url::parse(
                    "http://127.0.0.1:18080/realms/rvoip/protocol/openid-connect/token",
                )
                .unwrap(),
                authorization_endpoint: None,
                introspection_endpoint: introspection,
                revocation_endpoint: Some(
                    Url::parse(
                        "http://127.0.0.1:18080/realms/rvoip/protocol/openid-connect/revoke",
                    )
                    .unwrap(),
                ),
            },
            client: reqwest::Client::new(),
        }
    }

    #[test]
    fn introspection_validator_uses_discovered_endpoint() {
        let provider = test_provider(Some(
            Url::parse(
                "http://127.0.0.1:18080/realms/rvoip/protocol/openid-connect/token/introspect",
            )
            .unwrap(),
        ));

        provider
            .introspection_validator()
            .expect("introspection validator should be constructible");
    }

    #[test]
    fn introspection_validator_requires_discovered_endpoint() {
        let provider = test_provider(None);
        let err = match provider.introspection_validator() {
            Ok(_) => panic!("missing endpoint should fail clearly"),
            Err(err) => err,
        };

        assert!(
            matches!(err, KeycloakError::Config(message) if message.contains("introspection_endpoint"))
        );
    }
}
