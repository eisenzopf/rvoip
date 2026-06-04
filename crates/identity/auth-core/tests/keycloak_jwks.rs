//! Optional Keycloak integration test.
//!
//! Start the local fixture first:
//!
//!   cd /Users/jonathan/Developer/keycloak
//!   docker compose up -d
//!
//! Then run:
//!
//!   RVOIP_KEYCLOAK_BASE_URL=http://127.0.0.1:18080 \
//!     cargo test -p rvoip-auth-core --test keycloak_jwks

use rvoip_auth_core::{BearerValidator, JwksJwtValidator};
use rvoip_core_traits::identity::IdentityAssurance;
use serde::Deserialize;
use std::time::Duration;
use url::Url;

#[derive(Debug, Deserialize)]
struct TokenResponse {
    access_token: String,
}

#[tokio::test]
async fn keycloak_password_grant_token_validates_via_jwks() {
    let Ok(base_url) = std::env::var("RVOIP_KEYCLOAK_BASE_URL") else {
        eprintln!("skipping Keycloak integration; set RVOIP_KEYCLOAK_BASE_URL");
        return;
    };
    let base_url = base_url.trim_end_matches('/');
    let client = reqwest::Client::new();
    let token_url = format!("{base_url}/realms/rvoip/protocol/openid-connect/token");
    let token = fetch_token_with_retry(&client, &token_url).await;

    let jwks_url = Url::parse(&format!(
        "{base_url}/realms/rvoip/protocol/openid-connect/certs"
    ))
    .expect("valid JWKS URL");
    let issuer = format!("{base_url}/realms/rvoip");
    let validator = JwksJwtValidator::new(jwks_url)
        .with_issuer([issuer])
        .with_audience(["rvoip-sip"]);
    let assurance = validator
        .validate(&token)
        .await
        .expect("Keycloak token validates through JWKS");

    match assurance {
        IdentityAssurance::UserAuthorized { scopes, .. } => {
            assert!(
                scopes
                    .iter()
                    .any(|scope| scope == "profile" || scope == "email"),
                "expected Keycloak mapped profile/email scopes, got {scopes:?}"
            );
        }
        other => panic!("expected UserAuthorized, got {other:?}"),
    }
}

async fn fetch_token_with_retry(client: &reqwest::Client, token_url: &str) -> String {
    let mut last_error = String::new();
    for _ in 0..60 {
        match client
            .post(token_url)
            .form(&[
                ("grant_type", "password"),
                ("client_id", "rvoip-sip"),
                ("client_secret", "rvoip-sip-secret"),
                ("username", "alice"),
                ("password", "SecurePass2024"),
            ])
            .send()
            .await
        {
            Ok(response) => match response.error_for_status() {
                Ok(response) => match response.json::<TokenResponse>().await {
                    Ok(token) => return token.access_token,
                    Err(err) => last_error = err.to_string(),
                },
                Err(err) => last_error = err.to_string(),
            },
            Err(err) => last_error = err.to_string(),
        }
        tokio::time::sleep(Duration::from_secs(1)).await;
    }
    panic!("Keycloak token endpoint did not become ready: {last_error}");
}
