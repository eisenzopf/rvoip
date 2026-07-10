use chrono::Utc;
use rvoip_auth_core::{
    AuthenticationMethod, BearerAuthError, BearerValidator, OAuth2IntrospectionValidator,
};
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
async fn principal_preserves_introspection_authorization_claims() {
    let server = MockServer::start().await;
    let expires_at = (Utc::now() + chrono::Duration::hours(1)).timestamp();
    Mock::given(method("POST"))
        .and(path("/introspect"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "active": true,
            "sub": "shared-subject",
            "scope": "calls:read calls:write",
            "iss": "https://issuer.example",
            "tenant_id": "tenant-a",
            "exp": expires_at
        })))
        .mount(&server)
        .await;
    let validator =
        OAuth2IntrospectionValidator::new(format!("{}/introspect", server.uri()).parse().unwrap());

    let principal = validator.validate_principal("opaque-token").await.unwrap();

    assert_eq!(principal.subject, "shared-subject");
    assert_eq!(principal.tenant.as_deref(), Some("tenant-a"));
    assert_eq!(principal.issuer.as_deref(), Some("https://issuer.example"));
    assert_eq!(principal.scopes, vec!["calls:read", "calls:write"]);
    assert_eq!(principal.method, AuthenticationMethod::OAuth2Introspection);
    assert_eq!(
        principal.expires_at.expect("expiry").timestamp(),
        expires_at
    );
    assert!(!principal.is_expired());
    match principal.assurance {
        IdentityAssurance::UserAuthorized {
            identity, scopes, ..
        } => {
            assert_eq!(identity.as_str(), "shared-subject");
            assert_eq!(scopes, vec!["calls:read", "calls:write"]);
        }
        other => panic!("expected UserAuthorized, got {other:?}"),
    }
}

#[tokio::test]
async fn active_but_expired_introspection_response_rejects() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/introspect"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "active": true,
            "sub": "id_alice",
            "exp": (Utc::now() - chrono::Duration::minutes(1)).timestamp()
        })))
        .mount(&server)
        .await;
    let validator =
        OAuth2IntrospectionValidator::new(format!("{}/introspect", server.uri()).parse().unwrap());

    let result = validator.validate_principal("opaque-token").await;
    assert!(matches!(result, Err(BearerAuthError::Invalid(err)) if err.contains("expired")));
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
