//! SAML 2.0 service-provider adapter for RVoIP users-core.
//!
//! SAML is not a SIP authentication scheme. This crate is for enterprise web
//! SSO: validate an IdP assertion, link the external subject to a users-core
//! user, and issue users-core tokens for the application layer. XML signature
//! and IdP certificate validation are deliberately behind
//! [`SamlAssertionVerifier`] so deployments can use a reviewed SAML/XML
//! security implementation or a corporate SSO gateway.

use std::collections::{BTreeSet, HashSet};
use std::sync::Arc;

use async_trait::async_trait;
use chrono::{DateTime, Duration, Utc};
use parking_lot::Mutex;
use serde::{Deserialize, Serialize};
use thiserror::Error;
use users_core::{
    AuthenticationResult, AuthenticationService, CreateUserRequest, Error as UsersError,
    TokenIssueContext, UpdateUserRequest, UpsertExternalIdentityRequest, User,
};
use uuid::Uuid;

/// SAML adapter errors.
#[derive(Debug, Error)]
pub enum SamlError {
    #[error("SAML configuration error: {0}")]
    Config(String),
    #[error("SAML assertion rejected: {0}")]
    Rejected(String),
    #[error("users-core error: {0}")]
    Users(#[from] UsersError),
}

pub type Result<T> = std::result::Result<T, SamlError>;

/// Service-provider configuration.
#[derive(Debug, Clone)]
pub struct SamlServiceProviderConfig {
    /// Stable provider id used in users-core external identity links.
    pub provider_id: String,
    /// SP entity ID.
    pub entity_id: String,
    /// Assertion Consumer Service URL.
    pub acs_url: String,
    /// Required assertion audience. Defaults to `entity_id`.
    pub audience: String,
    /// Clock skew allowed for assertion time conditions.
    pub allowed_clock_skew: Duration,
    /// Default users-core roles for newly linked users.
    pub default_roles: Vec<String>,
}

impl SamlServiceProviderConfig {
    pub fn new(
        provider_id: impl Into<String>,
        entity_id: impl Into<String>,
        acs_url: impl Into<String>,
    ) -> Self {
        let entity_id = entity_id.into();
        Self {
            provider_id: provider_id.into(),
            audience: entity_id.clone(),
            entity_id,
            acs_url: acs_url.into(),
            allowed_clock_skew: Duration::minutes(5),
            default_roles: vec!["user".to_string()],
        }
    }

    pub fn with_audience(mut self, audience: impl Into<String>) -> Self {
        self.audience = audience.into();
        self
    }

    pub fn with_allowed_clock_skew(mut self, skew: Duration) -> Self {
        self.allowed_clock_skew = skew;
        self
    }

    pub fn with_default_roles(
        mut self,
        roles: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.default_roles = roles.into_iter().map(Into::into).collect();
        self
    }
}

/// Identity returned after a SAML verifier has checked signatures, issuer,
/// recipient, conditions, and IdP signing keys.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct VerifiedSamlIdentity {
    pub assertion_id: String,
    pub issuer: String,
    pub subject: String,
    pub audience: String,
    pub recipient: Option<String>,
    pub email: Option<String>,
    pub username: Option<String>,
    pub display_name: Option<String>,
    pub groups: Vec<String>,
    pub not_before: Option<DateTime<Utc>>,
    pub not_on_or_after: DateTime<Utc>,
}

/// Production SAML assertion verifier boundary.
///
/// Implementations must reject unsigned assertions/responses unless a
/// deployment explicitly accepts a non-production policy. They should validate
/// IdP metadata/certificate, issuer, audience, recipient/ACS URL, time
/// conditions, and XML signature wrapping protections before returning a
/// [`VerifiedSamlIdentity`].
#[async_trait]
pub trait SamlAssertionVerifier: Send + Sync {
    async fn verify_assertion(&self, saml_response: &str) -> Result<VerifiedSamlIdentity>;
}

/// SAML service-provider adapter backed by users-core token issuance.
#[derive(Clone)]
pub struct SamlServiceProvider {
    config: SamlServiceProviderConfig,
    auth_service: Arc<AuthenticationService>,
    verifier: Arc<dyn SamlAssertionVerifier>,
    replayed_assertions: Arc<Mutex<HashSet<String>>>,
}

impl SamlServiceProvider {
    pub fn new(
        config: SamlServiceProviderConfig,
        auth_service: Arc<AuthenticationService>,
        verifier: Arc<dyn SamlAssertionVerifier>,
    ) -> Result<Self> {
        if config.provider_id.trim().is_empty() {
            return Err(SamlError::Config("provider_id is required".to_string()));
        }
        if config.entity_id.trim().is_empty() {
            return Err(SamlError::Config("entity_id is required".to_string()));
        }
        if config.acs_url.trim().is_empty() {
            return Err(SamlError::Config("acs_url is required".to_string()));
        }
        if auth_service.enterprise_identity_store().is_none() {
            return Err(SamlError::Config(
                "AuthenticationService must be configured with an EnterpriseIdentityStore"
                    .to_string(),
            ));
        }
        Ok(Self {
            config,
            auth_service,
            verifier,
            replayed_assertions: Arc::new(Mutex::new(HashSet::new())),
        })
    }

    /// Minimal SP metadata document for IdP setup.
    pub fn metadata_xml(&self) -> String {
        format!(
            r#"<EntityDescriptor entityID="{entity_id}" xmlns="urn:oasis:names:tc:SAML:2.0:metadata"><SPSSODescriptor protocolSupportEnumeration="urn:oasis:names:tc:SAML:2.0:protocol"><AssertionConsumerService Binding="urn:oasis:names:tc:SAML:2.0:bindings:HTTP-POST" Location="{acs_url}" index="0" isDefault="true"/></SPSSODescriptor></EntityDescriptor>"#,
            entity_id = escape_xml(&self.config.entity_id),
            acs_url = escape_xml(&self.config.acs_url),
        )
    }

    /// Minimal AuthnRequest XML. Applications normally deflate/base64/sign
    /// this according to their chosen binding and IdP policy.
    pub fn authn_request_xml(&self, destination: &str) -> String {
        let id = format!("_{}", Uuid::new_v4().simple());
        let issue_instant = Utc::now().to_rfc3339();
        format!(
            r#"<samlp:AuthnRequest xmlns:samlp="urn:oasis:names:tc:SAML:2.0:protocol" ID="{id}" Version="2.0" IssueInstant="{issue_instant}" Destination="{destination}" AssertionConsumerServiceURL="{acs_url}"><saml:Issuer xmlns:saml="urn:oasis:names:tc:SAML:2.0:assertion">{entity_id}</saml:Issuer></samlp:AuthnRequest>"#,
            id = escape_xml(&id),
            issue_instant = escape_xml(&issue_instant),
            destination = escape_xml(destination),
            acs_url = escape_xml(&self.config.acs_url),
            entity_id = escape_xml(&self.config.entity_id),
        )
    }

    /// Validate a SAML response, link/create the user, and issue users-core
    /// tokens for the authenticated subject.
    pub async fn consume_assertion(&self, saml_response: &str) -> Result<AuthenticationResult> {
        let identity = self.verifier.verify_assertion(saml_response).await?;
        self.validate_verified_identity(&identity)?;
        self.mark_assertion_used(&identity.assertion_id)?;
        let user = self.link_or_create_user(&identity).await?;
        Ok(self
            .auth_service
            .issue_tokens_for_user(
                &user.id,
                TokenIssueContext::external_identity(
                    "saml",
                    &self.config.provider_id,
                    &identity.subject,
                ),
            )
            .await?)
    }

    pub fn config(&self) -> &SamlServiceProviderConfig {
        &self.config
    }

    fn validate_verified_identity(&self, identity: &VerifiedSamlIdentity) -> Result<()> {
        let now = Utc::now();
        if identity.audience != self.config.audience {
            return Err(SamlError::Rejected("audience mismatch".to_string()));
        }
        if let Some(recipient) = &identity.recipient {
            if recipient != &self.config.acs_url {
                return Err(SamlError::Rejected("recipient/ACS mismatch".to_string()));
            }
        }
        if let Some(not_before) = identity.not_before {
            if now + self.config.allowed_clock_skew < not_before {
                return Err(SamlError::Rejected(
                    "assertion is not yet valid".to_string(),
                ));
            }
        }
        if now - self.config.allowed_clock_skew >= identity.not_on_or_after {
            return Err(SamlError::Rejected("assertion has expired".to_string()));
        }
        Ok(())
    }

    fn mark_assertion_used(&self, assertion_id: &str) -> Result<()> {
        let mut used = self.replayed_assertions.lock();
        if !used.insert(assertion_id.to_string()) {
            return Err(SamlError::Rejected("assertion replay detected".to_string()));
        }
        Ok(())
    }

    async fn link_or_create_user(&self, identity: &VerifiedSamlIdentity) -> Result<User> {
        let store = self
            .auth_service
            .enterprise_identity_store()
            .ok_or_else(|| {
                SamlError::Config("EnterpriseIdentityStore is not configured".to_string())
            })?;
        let user = if let Some(link) = store
            .get_external_identity(&self.config.provider_id, &identity.subject)
            .await?
        {
            self.auth_service
                .user_store()
                .update_user(
                    &link.user_id,
                    UpdateUserRequest {
                        email: identity.email.clone(),
                        display_name: identity.display_name.clone(),
                        roles: Some(self.roles_from_groups(identity)),
                        active: Some(true),
                    },
                )
                .await?
        } else {
            let username = identity
                .username
                .clone()
                .or_else(|| identity.email.clone())
                .unwrap_or_else(|| identity.subject.clone());
            match self
                .auth_service
                .user_store()
                .get_user_by_username(&username)
                .await?
            {
                Some(existing) => {
                    self.auth_service
                        .user_store()
                        .update_user(
                            &existing.id,
                            UpdateUserRequest {
                                email: identity.email.clone(),
                                display_name: identity.display_name.clone(),
                                roles: Some(self.roles_from_groups(identity)),
                                active: Some(true),
                            },
                        )
                        .await?
                }
                None => {
                    self.auth_service
                        .create_user(CreateUserRequest {
                            username,
                            password: format!("SamlProvisioned!{}Aa1", Uuid::new_v4().simple()),
                            email: identity.email.clone(),
                            display_name: identity.display_name.clone(),
                            roles: self.roles_from_groups(identity),
                        })
                        .await?
                }
            }
        };
        store
            .upsert_external_identity(UpsertExternalIdentityRequest {
                provider_id: self.config.provider_id.clone(),
                external_subject: identity.subject.clone(),
                user_id: user.id.clone(),
                email: identity.email.clone(),
                username: identity
                    .username
                    .clone()
                    .or_else(|| Some(user.username.clone())),
                display_name: identity.display_name.clone(),
                groups: identity.groups.clone(),
                active: true,
            })
            .await?;
        Ok(user)
    }

    fn roles_from_groups(&self, identity: &VerifiedSamlIdentity) -> Vec<String> {
        let allowed = ["user", "admin", "moderator", "guest"];
        let mut roles = BTreeSet::new();
        for role in &self.config.default_roles {
            if allowed.contains(&role.as_str()) {
                roles.insert(role.clone());
            }
        }
        for group in &identity.groups {
            if allowed.contains(&group.as_str()) {
                roles.insert(group.clone());
            }
        }
        if roles.is_empty() {
            roles.insert("user".to_string());
        }
        roles.into_iter().collect()
    }
}

fn escape_xml(value: &str) -> String {
    value
        .replace('&', "&amp;")
        .replace('<', "&lt;")
        .replace('>', "&gt;")
        .replace('"', "&quot;")
        .replace('\'', "&apos;")
}

#[cfg(test)]
mod tests {
    use super::*;
    use users_core::config::{PasswordConfig, TlsSettings};
    use users_core::jwt::JwtConfig;
    use users_core::{init, UsersConfig};

    #[derive(Clone)]
    struct StaticVerifier {
        identity: VerifiedSamlIdentity,
    }

    #[async_trait]
    impl SamlAssertionVerifier for StaticVerifier {
        async fn verify_assertion(&self, _saml_response: &str) -> Result<VerifiedSamlIdentity> {
            Ok(self.identity.clone())
        }
    }

    fn test_config(db_url: String) -> UsersConfig {
        UsersConfig {
            database_url: db_url,
            jwt: JwtConfig {
                issuer: "https://users.rvoip.local".to_string(),
                audience: vec!["rvoip-app".to_string()],
                access_ttl_seconds: 300,
                refresh_ttl_seconds: 3600,
                algorithm: "HS256".to_string(),
                signing_key: Some("saml-test-secret".to_string()),
            },
            password: PasswordConfig {
                min_length: 12,
                require_uppercase: true,
                require_lowercase: true,
                require_numbers: true,
                require_special: false,
                argon2_memory_cost: 1024,
                argon2_time_cost: 2,
                argon2_parallelism: 1,
            },
            api_bind_address: "127.0.0.1:0".to_string(),
            tls: TlsSettings::default(),
        }
    }

    async fn test_service(
        identity: VerifiedSamlIdentity,
    ) -> (tempfile::TempDir, SamlServiceProvider) {
        let temp = tempfile::tempdir().unwrap();
        let db_url = format!(
            "sqlite://{}?mode=rwc",
            temp.path().join("users.db").display()
        );
        let auth = Arc::new(init(test_config(db_url)).await.unwrap());
        let config = SamlServiceProviderConfig::new(
            "keycloak-saml",
            "urn:rvoip:test-sp",
            "https://app.example.test/saml/acs",
        );
        let service =
            SamlServiceProvider::new(config, auth, Arc::new(StaticVerifier { identity })).unwrap();
        (temp, service)
    }

    fn valid_identity(assertion_id: &str) -> VerifiedSamlIdentity {
        VerifiedSamlIdentity {
            assertion_id: assertion_id.to_string(),
            issuer: "https://idp.example.test".to_string(),
            subject: "subject-1".to_string(),
            audience: "urn:rvoip:test-sp".to_string(),
            recipient: Some("https://app.example.test/saml/acs".to_string()),
            email: Some("alice@example.test".to_string()),
            username: Some("alice".to_string()),
            display_name: Some("Alice Example".to_string()),
            groups: vec!["admin".to_string()],
            not_before: Some(Utc::now() - Duration::minutes(1)),
            not_on_or_after: Utc::now() + Duration::minutes(5),
        }
    }

    #[tokio::test]
    async fn consume_assertion_links_identity_and_issues_tokens() {
        let (_temp, service) = test_service(valid_identity("assertion-1")).await;
        let result = service.consume_assertion("<signed/>").await.unwrap();
        assert_eq!(result.user.username, "alice");
        assert!(!result.access_token.is_empty());

        let link = service
            .auth_service
            .enterprise_identity_store()
            .unwrap()
            .get_external_identity("keycloak-saml", "subject-1")
            .await
            .unwrap()
            .unwrap();
        assert_eq!(link.user_id, result.user.id);
    }

    #[tokio::test]
    async fn assertion_replay_is_rejected() {
        let (_temp, service) = test_service(valid_identity("assertion-replay")).await;
        service.consume_assertion("<signed/>").await.unwrap();
        let replay = service.consume_assertion("<signed/>").await;
        assert!(matches!(replay, Err(SamlError::Rejected(ref msg)) if msg.contains("replay")));
    }

    #[tokio::test]
    async fn wrong_audience_is_rejected() {
        let mut identity = valid_identity("assertion-wrong-audience");
        identity.audience = "wrong-audience".to_string();
        let (_temp, service) = test_service(identity).await;
        let rejected = service.consume_assertion("<signed/>").await;
        assert!(matches!(rejected, Err(SamlError::Rejected(ref msg)) if msg.contains("audience")));
    }
}
