//! SAML SSO into users-core example.
//!
//! This example uses a fake verifier that represents an already verified,
//! signed SAML assertion. Production code should implement
//! `SamlAssertionVerifier` using a reviewed SAML/XML signature validator.
//!
//! Run with:
//!
//!   cargo run -p rvoip-sip --example auth_saml_users_core

use std::sync::Arc;

use async_trait::async_trait;
use chrono::{Duration, Utc};
use rvoip_saml::{
    Result as SamlResult, SamlAssertionVerifier, SamlServiceProvider, SamlServiceProviderConfig,
    VerifiedSamlIdentity,
};
use tempfile::TempDir;
use users_core::{init, UsersConfig};

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let temp_dir = TempDir::new()?;
    let users = init(UsersConfig {
        database_url: format!(
            "sqlite://{}?mode=rwc",
            temp_dir.path().join("users.db").display()
        ),
        ..UsersConfig::default()
    })
    .await?;

    let sp = SamlServiceProvider::new(
        SamlServiceProviderConfig::new(
            "keycloak-saml",
            "urn:rvoip:example-sp",
            "https://app.example.test/saml/acs",
        ),
        Arc::new(users),
        Arc::new(ExampleVerifiedAssertion),
    )?;

    let login = sp.consume_assertion("<signed-saml-response/>").await?;
    println!(
        "SAML subject linked to users-core user {} and issued token expiring in {:?}",
        login.user.username, login.expires_in
    );
    Ok(())
}

struct ExampleVerifiedAssertion;

#[async_trait]
impl SamlAssertionVerifier for ExampleVerifiedAssertion {
    async fn verify_assertion(&self, _saml_response: &str) -> SamlResult<VerifiedSamlIdentity> {
        Ok(VerifiedSamlIdentity {
            assertion_id: "assertion-1".to_string(),
            issuer: "https://idp.example.test".to_string(),
            subject: "saml-subject-alice".to_string(),
            audience: "urn:rvoip:example-sp".to_string(),
            recipient: Some("https://app.example.test/saml/acs".to_string()),
            email: Some("alice@example.test".to_string()),
            username: Some("alice".to_string()),
            display_name: Some("Alice Example".to_string()),
            groups: vec!["user".to_string()],
            not_before: Some(Utc::now() - Duration::minutes(1)),
            not_on_or_after: Utc::now() + Duration::minutes(5),
        })
    }
}
