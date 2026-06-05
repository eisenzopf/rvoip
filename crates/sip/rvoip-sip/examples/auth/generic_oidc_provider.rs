//! Optional generic OIDC Bearer provider example.
//!
//! Run with:
//!
//!   RVOIP_OIDC_ISSUER=https://idp.example.com/realms/rvoip \
//!   RVOIP_OIDC_AUDIENCE=rvoip-sip \
//!     cargo run -p rvoip-sip --example auth_generic_oidc_provider

use std::sync::Arc;
use std::time::Duration;

use rvoip_oidc::OidcConfig;
use rvoip_sip::{SipAuthDecision, SipAuthService, SipAuthSource};
use url::Url;

#[tokio::main]
async fn main() -> anyhow::Result<()> {
    let Some(issuer) = std::env::var("RVOIP_OIDC_ISSUER").ok() else {
        println!("Skipping generic OIDC example; set RVOIP_OIDC_ISSUER.");
        return Ok(());
    };

    let mut config =
        OidcConfig::new(Url::parse(&issuer)?).with_jwks_cache_ttl(Duration::from_secs(300));
    if let Ok(audience) = std::env::var("RVOIP_OIDC_AUDIENCE") {
        config = config.with_audience(audience);
    }
    if let (Ok(client_id), Ok(client_secret)) = (
        std::env::var("RVOIP_OIDC_CLIENT_ID"),
        std::env::var("RVOIP_OIDC_CLIENT_SECRET"),
    ) {
        config = config.with_client_credentials(client_id, client_secret);
    }

    let provider = config.discover().await?;
    let health = provider.health_check().await?;
    println!(
        "OIDC issuer={} jwks_reachable={} introspection={:?}",
        health.issuer, health.jwks_reachable, health.introspection_endpoint
    );

    let auth =
        SipAuthService::new().with_bearer_validator("oidc", Arc::new(provider.bearer_validator()?));

    if let Ok(token) = std::env::var("RVOIP_OIDC_ACCESS_TOKEN") {
        match auth
            .authenticate_authorization(
                Some(&format!("Bearer {token}")),
                "REGISTER",
                "sip:pbx.example.com",
                None,
                SipAuthSource::Origin,
                true,
            )
            .await?
        {
            SipAuthDecision::Authorized(identity) => {
                println!("Bearer authorized subject={:?}", identity.subject);
            }
            SipAuthDecision::Rejected { challenges } => {
                println!("Bearer rejected with {} challenge(s)", challenges.len());
            }
        }
    } else {
        println!("Set RVOIP_OIDC_ACCESS_TOKEN to validate a live token.");
    }

    if provider.introspection_validator().is_ok() {
        println!("Provider metadata includes an OAuth2 introspection endpoint.");
    }

    Ok(())
}
