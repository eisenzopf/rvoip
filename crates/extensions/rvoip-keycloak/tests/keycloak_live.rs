use rvoip_auth_core::BearerValidator;
use rvoip_core_traits::identity::IdentityAssurance;
use rvoip_keycloak::{KeycloakConfig, KeycloakPasswordGrantClient};
use std::time::Duration;
use url::Url;

#[tokio::test]
async fn keycloak_fixture_token_validates_with_library_fixture() {
    let Ok(base_url) = std::env::var("RVOIP_KEYCLOAK_BASE_URL") else {
        eprintln!("skipping Keycloak integration; set RVOIP_KEYCLOAK_BASE_URL");
        return;
    };
    let config = KeycloakConfig::new(
        Url::parse(&base_url).expect("valid Keycloak base URL"),
        std::env::var("RVOIP_KEYCLOAK_REALM").unwrap_or_else(|_| "rvoip".to_string()),
        std::env::var("RVOIP_KEYCLOAK_CLIENT_ID").unwrap_or_else(|_| "rvoip-sip".to_string()),
    )
    .with_client_secret(
        std::env::var("RVOIP_KEYCLOAK_CLIENT_SECRET")
            .unwrap_or_else(|_| "rvoip-sip-secret".to_string()),
    )
    .with_audience(
        std::env::var("RVOIP_KEYCLOAK_CLIENT_ID").unwrap_or_else(|_| "rvoip-sip".to_string()),
    );

    let username =
        std::env::var("RVOIP_KEYCLOAK_TEST_USERNAME").unwrap_or_else(|_| "alice".to_string());
    let password = std::env::var("RVOIP_KEYCLOAK_TEST_PASSWORD")
        .unwrap_or_else(|_| "SecurePass2024".to_string());

    let provider = config
        .discover()
        .await
        .expect("Keycloak OIDC discovery should succeed");
    let health = provider
        .health_check()
        .await
        .expect("Keycloak JWKS endpoint should be reachable");
    assert!(health.jwks_reachable);
    assert_eq!(health.audience.as_deref(), Some(config.client_id.as_str()));
    assert!(
        health.introspection_endpoint.is_some(),
        "Keycloak should advertise an introspection endpoint"
    );
    provider
        .introspection_validator()
        .expect("introspection validator from OIDC discovery metadata");

    let token_client = KeycloakPasswordGrantClient::new(config.clone());
    let token = token_client
        .access_token_with_retry(&username, &password, 60, Duration::from_secs(1))
        .await
        .expect("Keycloak fixture should issue token");
    let validator = provider
        .bearer_validator()
        .expect("validator from OIDC discovery metadata");
    let assurance = validator
        .validate(&token)
        .await
        .expect("Keycloak token validates through library fixture");

    match assurance {
        IdentityAssurance::UserAuthorized { scopes, .. } => {
            assert!(!scopes.is_empty(), "expected scopes from Keycloak token");
        }
        other => panic!("expected UserAuthorized, got {other:?}"),
    }
}
