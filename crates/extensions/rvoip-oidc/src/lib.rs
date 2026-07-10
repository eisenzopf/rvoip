//! Generic OIDC discovery helpers for RVoIP Bearer authentication.
//!
//! This crate is optional. It turns OIDC discovery metadata into
//! `rvoip-auth-core` Bearer validators while keeping SIP protocol crates
//! dependent only on auth-core traits.

use std::sync::Arc;
use std::time::Duration;

use async_trait::async_trait;
use rvoip_auth_core::{
    AuthenticatedPrincipal, BearerAuthError, BearerValidator, JwksJwtValidator,
    OAuth2IntrospectionValidator,
};
use rvoip_core_traits::identity::IdentityAssurance;
use serde::Deserialize;
use thiserror::Error;
use url::Url;

/// Generic OIDC provider configuration.
#[derive(Debug, Clone)]
pub struct OidcConfig {
    /// Provider issuer URL.
    pub issuer: Url,
    /// Expected audience/resource indicator for access tokens.
    pub audience: Option<String>,
    /// OAuth2 client id used for token introspection client auth.
    pub client_id: Option<String>,
    /// OAuth2 client secret used for token introspection client auth.
    pub client_secret: Option<String>,
    /// JWKS cache TTL for validators created from this config.
    pub jwks_cache_ttl: Option<Duration>,
}

impl OidcConfig {
    /// Create an OIDC config from an issuer URL.
    pub fn new(issuer: Url) -> Self {
        Self {
            issuer,
            audience: None,
            client_id: None,
            client_secret: None,
            jwks_cache_ttl: None,
        }
    }

    /// Set expected token audience.
    pub fn with_audience(mut self, audience: impl Into<String>) -> Self {
        self.audience = Some(audience.into());
        self
    }

    /// Set client credentials used for OAuth2 introspection.
    pub fn with_client_credentials(
        mut self,
        client_id: impl Into<String>,
        client_secret: impl Into<String>,
    ) -> Self {
        self.client_id = Some(client_id.into());
        self.client_secret = Some(client_secret.into());
        self
    }

    /// Set JWKS cache TTL.
    pub fn with_jwks_cache_ttl(mut self, ttl: Duration) -> Self {
        self.jwks_cache_ttl = Some(ttl);
        self
    }

    /// OIDC discovery URL for this issuer.
    pub fn discovery_url(&self) -> Result<Url, OidcError> {
        let mut url = self.issuer.clone();
        let base_path = url.path().trim_end_matches('/');
        url.set_path(&format!("{base_path}/.well-known/openid-configuration"));
        Ok(url)
    }

    /// Discover provider metadata.
    pub async fn discover(&self) -> Result<OidcProvider, OidcError> {
        OidcProvider::discover(self.clone()).await
    }
}

/// OIDC discovery metadata used by RVoIP validators.
#[derive(Debug, Clone, Deserialize)]
pub struct OidcProviderMetadata {
    /// Token issuer.
    pub issuer: String,
    /// JWKS endpoint.
    pub jwks_uri: Url,
    /// Token endpoint, when advertised.
    #[serde(default)]
    pub token_endpoint: Option<Url>,
    /// Introspection endpoint, when advertised.
    #[serde(default)]
    pub introspection_endpoint: Option<Url>,
    /// Revocation endpoint, when advertised.
    #[serde(default)]
    pub revocation_endpoint: Option<Url>,
    /// Authorization endpoint, when advertised.
    #[serde(default)]
    pub authorization_endpoint: Option<Url>,
}

/// Discovered generic OIDC provider.
#[derive(Debug, Clone)]
pub struct OidcProvider {
    config: OidcConfig,
    metadata: OidcProviderMetadata,
    client: reqwest::Client,
}

impl OidcProvider {
    /// Discover OIDC provider metadata from `/.well-known/openid-configuration`.
    pub async fn discover(config: OidcConfig) -> Result<Self, OidcError> {
        let client = reqwest::Client::new();
        let metadata = client
            .get(config.discovery_url()?)
            .send()
            .await?
            .error_for_status()?
            .json::<OidcProviderMetadata>()
            .await?;
        Self::from_metadata(config, metadata, client)
    }

    /// Build a provider from already-discovered metadata.
    pub fn from_metadata(
        config: OidcConfig,
        metadata: OidcProviderMetadata,
        client: reqwest::Client,
    ) -> Result<Self, OidcError> {
        if metadata.issuer != config.issuer.to_string().trim_end_matches('/') {
            return Err(OidcError::Config(format!(
                "discovered issuer {} does not match configured issuer {}",
                metadata.issuer, config.issuer
            )));
        }
        Ok(Self {
            config,
            metadata,
            client,
        })
    }

    /// Return discovered metadata.
    pub fn metadata(&self) -> &OidcProviderMetadata {
        &self.metadata
    }

    /// Build a JWKS-backed Bearer validator.
    pub fn bearer_validator(&self) -> Result<OidcBearerValidator, OidcError> {
        let mut validator =
            JwksJwtValidator::new(self.metadata.jwks_uri.clone()).with_issuer([self.issuer()]);
        if let Some(ttl) = self.config.jwks_cache_ttl {
            validator = validator.with_cache_ttl(ttl);
        }
        if let Some(audience) = self.config.audience.as_ref() {
            validator = validator.with_audience([audience.as_str()]);
        }
        Ok(OidcBearerValidator {
            inner: Arc::new(validator),
        })
    }

    /// Build an OAuth2 introspection Bearer validator.
    pub fn introspection_validator(&self) -> Result<OAuth2IntrospectionValidator, OidcError> {
        let endpoint = self
            .metadata
            .introspection_endpoint
            .clone()
            .ok_or_else(|| {
                OidcError::Config(
                    "OIDC provider metadata does not include introspection_endpoint".to_string(),
                )
            })?;
        let mut validator =
            OAuth2IntrospectionValidator::new(endpoint).with_issuer([self.issuer()]);
        if let Some(audience) = self.config.audience.as_ref() {
            validator = validator.with_audience([audience.as_str()]);
        }
        if let (Some(client_id), Some(client_secret)) = (
            self.config.client_id.as_ref(),
            self.config.client_secret.as_ref(),
        ) {
            validator = validator.with_basic_client_auth(client_id.clone(), client_secret.clone());
        }
        Ok(validator)
    }

    /// Check that the discovered JWKS endpoint is reachable.
    pub async fn health_check(&self) -> Result<OidcHealth, OidcError> {
        self.client
            .get(self.metadata.jwks_uri.clone())
            .send()
            .await?
            .error_for_status()?;
        Ok(OidcHealth {
            issuer: self.metadata.issuer.clone(),
            jwks_uri: self.metadata.jwks_uri.clone(),
            jwks_reachable: true,
            introspection_endpoint: self.metadata.introspection_endpoint.clone(),
            revocation_endpoint: self.metadata.revocation_endpoint.clone(),
            audience: self.config.audience.clone(),
        })
    }

    fn issuer(&self) -> &str {
        self.metadata.issuer.as_str()
    }
}

/// Generic OIDC provider health result.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct OidcHealth {
    /// Issuer from metadata.
    pub issuer: String,
    /// JWKS endpoint.
    pub jwks_uri: Url,
    /// Whether JWKS endpoint was reachable.
    pub jwks_reachable: bool,
    /// Optional introspection endpoint.
    pub introspection_endpoint: Option<Url>,
    /// Optional revocation endpoint.
    pub revocation_endpoint: Option<Url>,
    /// Configured expected audience.
    pub audience: Option<String>,
}

/// JWKS-backed generic OIDC Bearer validator.
#[derive(Clone)]
pub struct OidcBearerValidator {
    inner: Arc<dyn BearerValidator>,
}

#[async_trait]
impl BearerValidator for OidcBearerValidator {
    async fn validate(&self, token: &str) -> Result<IdentityAssurance, BearerAuthError> {
        self.inner.validate(token).await
    }

    async fn validate_principal(
        &self,
        token: &str,
    ) -> Result<AuthenticatedPrincipal, BearerAuthError> {
        self.inner.validate_principal(token).await
    }
}

/// Errors from generic OIDC configuration and discovery.
#[derive(Debug, Error)]
pub enum OidcError {
    /// HTTP client error.
    #[error("OIDC HTTP error: {0}")]
    Http(#[from] reqwest::Error),
    /// Configuration error.
    #[error("OIDC configuration error: {0}")]
    Config(String),
}

#[cfg(test)]
mod tests {
    use super::*;

    struct RichValidator;

    #[async_trait]
    impl BearerValidator for RichValidator {
        async fn validate(&self, _token: &str) -> Result<IdentityAssurance, BearerAuthError> {
            Ok(self.validate_principal("token").await?.assurance)
        }

        async fn validate_principal(
            &self,
            _token: &str,
        ) -> Result<AuthenticatedPrincipal, BearerAuthError> {
            let identity = rvoip_core_traits::ids::IdentityId::from_string("subject-a");
            Ok(AuthenticatedPrincipal {
                subject: "subject-a".into(),
                tenant: Some("tenant-a".into()),
                scopes: vec!["calls:read".into()],
                issuer: Some("https://issuer.example".into()),
                expires_at: None,
                method: rvoip_auth_core::AuthenticationMethod::Jwt,
                assurance: IdentityAssurance::UserAuthorized {
                    identity: identity.clone(),
                    user_id: identity,
                    scopes: vec!["calls:read".into()],
                },
            })
        }
    }

    fn metadata() -> OidcProviderMetadata {
        OidcProviderMetadata {
            issuer: "https://idp.example.test/realms/rvoip".to_string(),
            jwks_uri: Url::parse("https://idp.example.test/realms/rvoip/certs").unwrap(),
            token_endpoint: Some(
                Url::parse("https://idp.example.test/realms/rvoip/token").unwrap(),
            ),
            introspection_endpoint: Some(
                Url::parse("https://idp.example.test/realms/rvoip/introspect").unwrap(),
            ),
            revocation_endpoint: None,
            authorization_endpoint: None,
        }
    }

    #[test]
    fn discovery_url_uses_issuer_path() {
        let config = OidcConfig::new(Url::parse("https://idp.example.test/realms/rvoip").unwrap());
        assert_eq!(
            config.discovery_url().unwrap().as_str(),
            "https://idp.example.test/realms/rvoip/.well-known/openid-configuration"
        );
    }

    #[test]
    fn builds_validators_from_metadata() {
        let config = OidcConfig::new(Url::parse("https://idp.example.test/realms/rvoip").unwrap())
            .with_audience("rvoip-sip")
            .with_client_credentials("sip-client", "secret")
            .with_jwks_cache_ttl(Duration::from_secs(60));
        let provider =
            OidcProvider::from_metadata(config, metadata(), reqwest::Client::new()).unwrap();

        let _jwks = provider.bearer_validator().unwrap();
        let _introspection = provider.introspection_validator().unwrap();
    }

    #[test]
    fn missing_introspection_endpoint_is_clear_config_error() {
        let config = OidcConfig::new(Url::parse("https://idp.example.test/realms/rvoip").unwrap());
        let mut metadata = metadata();
        metadata.introspection_endpoint = None;
        let provider =
            OidcProvider::from_metadata(config, metadata, reqwest::Client::new()).unwrap();

        let error = provider
            .introspection_validator()
            .err()
            .expect("missing introspection should error");
        assert!(error.to_string().contains("introspection_endpoint"));
    }

    #[test]
    fn issuer_mismatch_rejects_metadata() {
        let config = OidcConfig::new(Url::parse("https://idp.example.test/other").unwrap());
        let error = OidcProvider::from_metadata(config, metadata(), reqwest::Client::new())
            .expect_err("issuer mismatch should fail");
        assert!(error.to_string().contains("does not match"));
    }

    #[tokio::test]
    async fn wrapper_preserves_rich_principal() {
        let wrapper = OidcBearerValidator {
            inner: Arc::new(RichValidator),
        };
        let principal = wrapper.validate_principal("token").await.unwrap();
        assert_eq!(principal.subject, "subject-a");
        assert_eq!(principal.tenant.as_deref(), Some("tenant-a"));
        assert_eq!(principal.issuer.as_deref(), Some("https://issuer.example"));
        assert_eq!(principal.method, rvoip_auth_core::AuthenticationMethod::Jwt);
    }
}
