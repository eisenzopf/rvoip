use rvoip_auth_core::{BearerAuthError, BearerValidator, OAuth2IntrospectionValidator};
use rvoip_core_traits::identity::IdentityAssurance;
use serde_json::json;
use wiremock::matchers::{method, path};
use wiremock::{Mock, MockServer, ResponseTemplate};

#[tokio::test]
async fn active_introspection_response_authorizes_user() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/introspect"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "active": true,
            "sub": "id_alice",
            "scope": "sip.register sip.call",
            "iss": "https://idp.example.com",
            "aud": ["rvoip-sip"]
        })))
        .mount(&server)
        .await;

    let validator =
        OAuth2IntrospectionValidator::new(format!("{}/introspect", server.uri()).parse().unwrap())
            .with_issuer(["https://idp.example.com"])
            .with_audience(["rvoip-sip"]);

    let assurance = validator.validate("opaque-token").await.unwrap();

    match assurance {
        IdentityAssurance::UserAuthorized {
            user_id, scopes, ..
        } => {
            assert_eq!(user_id.as_str(), "id_alice");
            assert_eq!(scopes, vec!["sip.register", "sip.call"]);
        }
        other => panic!("expected UserAuthorized, got {other:?}"),
    }
}

#[tokio::test]
async fn inactive_introspection_response_rejects() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/introspect"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "active": false
        })))
        .mount(&server)
        .await;

    let validator =
        OAuth2IntrospectionValidator::new(format!("{}/introspect", server.uri()).parse().unwrap());

    let result = validator.validate("opaque-token").await;
    assert!(matches!(result, Err(BearerAuthError::Invalid(err)) if err.contains("inactive")));
}

#[tokio::test]
async fn issuer_or_audience_mismatch_rejects() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/introspect"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "active": true,
            "sub": "id_alice",
            "iss": "https://wrong.example.com",
            "aud": "wrong-audience"
        })))
        .mount(&server)
        .await;

    let validator =
        OAuth2IntrospectionValidator::new(format!("{}/introspect", server.uri()).parse().unwrap())
            .with_issuer(["https://idp.example.com"])
            .with_audience(["rvoip-sip"]);

    let result = validator.validate("opaque-token").await;
    assert!(matches!(result, Err(BearerAuthError::Invalid(err)) if err.contains("issuer")));
}

#[tokio::test]
async fn audience_mismatch_rejects_when_issuer_matches() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/introspect"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "active": true,
            "sub": "id_alice",
            "iss": "https://idp.example.com",
            "aud": ["wrong-audience"]
        })))
        .mount(&server)
        .await;

    let validator =
        OAuth2IntrospectionValidator::new(format!("{}/introspect", server.uri()).parse().unwrap())
            .with_issuer(["https://idp.example.com"])
            .with_audience(["rvoip-sip"]);

    let result = validator.validate("opaque-token").await;
    assert!(matches!(result, Err(BearerAuthError::Invalid(err)) if err.contains("audience")));
}
