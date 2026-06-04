//! Optional Keycloak/OIDC Bearer provider example.
//!
//! This example uses the `rvoip-keycloak` extension crate to discover a
//! Keycloak realm, build a JWKS-backed Bearer validator, issue a local fixture
//! token with the password-grant demo client, and authenticate that token
//! through `SipAuthService`.
//!
//! It exits successfully when Keycloak configuration is not available. This
//! keeps `cargo run --example ...` usable for developers who do not have the
//! optional local fixture running.
//!
//! Run with an existing fixture:
//!
//!   . ~/Developer/keycloak/keycloak-local.env
//!   cargo run -p rvoip-sip --example auth_keycloak_bearer_provider
//!
//! Or point at an env file explicitly:
//!
//!   RVOIP_KEYCLOAK_ENV=~/Developer/keycloak/keycloak-local.env \
//!     cargo run -p rvoip-sip --example auth_keycloak_bearer_provider

use std::path::PathBuf;
use std::time::Duration;

use rvoip_keycloak::{KeycloakConfig, KeycloakPasswordGrantClient};
use rvoip_sip::{SipAuthDecision, SipAuthService, SipAuthSource};
use url::Url;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    load_optional_keycloak_env();

    let Some(base_url) = std::env::var("RVOIP_KEYCLOAK_BASE_URL").ok() else {
        println!("Skipping Keycloak example; set RVOIP_KEYCLOAK_BASE_URL or RVOIP_KEYCLOAK_ENV.");
        println!("Local fixture hint: . ~/Developer/keycloak/keycloak-local.env");
        return Ok(());
    };

    let realm = std::env::var("RVOIP_KEYCLOAK_REALM").unwrap_or_else(|_| "rvoip".to_string());
    let client_id =
        std::env::var("RVOIP_KEYCLOAK_CLIENT_ID").unwrap_or_else(|_| "rvoip-sip".to_string());
    let client_secret = std::env::var("RVOIP_KEYCLOAK_CLIENT_SECRET")
        .unwrap_or_else(|_| "rvoip-sip-secret".to_string());
    let username =
        std::env::var("RVOIP_KEYCLOAK_TEST_USERNAME").unwrap_or_else(|_| "alice".to_string());
    let password = std::env::var("RVOIP_KEYCLOAK_TEST_PASSWORD")
        .unwrap_or_else(|_| "SecurePass2024".to_string());

    let config = KeycloakConfig::new(Url::parse(&base_url)?, realm, client_id.clone())
        .with_client_secret(client_secret)
        .with_audience(client_id)
        .with_jwks_cache_ttl(Duration::from_secs(300));

    let provider = config.discover().await?;
    let health = provider.health_check().await?;
    println!(
        "Keycloak issuer={} jwks_reachable={} introspection={:?}",
        health.issuer, health.jwks_reachable, health.introspection_endpoint
    );

    let token = KeycloakPasswordGrantClient::new(config)
        .access_token_with_retry(&username, &password, 10, Duration::from_millis(500))
        .await?;

    let bearer_validator = provider.bearer_validator()?.into_arc();
    let auth = SipAuthService::new()
        .with_bearer_validator("keycloak", bearer_validator)
        .with_bearer_scope("sip.register sip.call");

    let decision = auth
        .authenticate_authorization(
            Some(&format!("Bearer {token}")),
            "REGISTER",
            "sip:pbx.example.com",
            None,
            SipAuthSource::Origin,
            true,
        )
        .await?;

    match decision {
        SipAuthDecision::Authorized(identity) => {
            println!(
                "Authorized via Keycloak subject={:?} scopes={:?}",
                identity.subject, identity.scopes
            );
        }
        SipAuthDecision::Rejected { challenges } => {
            println!(
                "Token was rejected; generated {} challenge(s)",
                challenges.len()
            );
        }
    }

    if provider.introspection_validator().is_ok() {
        println!("Keycloak discovery also provided an OAuth2 introspection endpoint.");
    }

    Ok(())
}

fn load_optional_keycloak_env() {
    if let Ok(path) = std::env::var("RVOIP_KEYCLOAK_ENV") {
        let _ = dotenvy::from_path(path);
        return;
    }

    let Some(home) = std::env::var_os("HOME") else {
        return;
    };
    let path = PathBuf::from(home)
        .join("Developer")
        .join("keycloak")
        .join("keycloak-local.env");
    if path.exists() {
        let _ = dotenvy::from_path(path);
    }
}
