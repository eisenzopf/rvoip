use chrono::Utc;
use rvoip_auth_core::{
    AuthenticationMethod, BearerAuthError, BearerValidator, OAuth2IntrospectionValidator,
};
use rvoip_core_traits::identity::IdentityAssurance;
use serde_json::json;
use std::time::{Duration, UNIX_EPOCH};
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
async fn credential_preserves_provider_token_id_alias_and_iat() {
    let server = MockServer::start().await;
    let issued_at = u64::try_from(Utc::now().timestamp()).unwrap() - 30;
    Mock::given(method("POST"))
        .and(path("/introspect"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "active": true,
            "sub": "id_alice",
            "token_id": "opaque-id-123",
            "iat": issued_at,
            "exp": (Utc::now() + chrono::Duration::hours(1)).timestamp()
        })))
        .mount(&server)
        .await;
    let validator =
        OAuth2IntrospectionValidator::new(format!("{}/introspect", server.uri()).parse().unwrap());

    let credential = validator.validate_credential("opaque-token").await.unwrap();
    assert_eq!(credential.token_id.as_deref(), Some("opaque-id-123"));
    assert_eq!(
        credential.issued_at,
        Some(UNIX_EPOCH + Duration::from_secs(issued_at))
    );
}

#[tokio::test]
async fn missing_provider_token_id_uses_stable_redacted_fingerprint() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/introspect"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "active": true,
            "sub": "id_alice"
        })))
        .mount(&server)
        .await;
    let validator =
        OAuth2IntrospectionValidator::new(format!("{}/introspect", server.uri()).parse().unwrap());

    let first = validator.validate_credential("opaque-a").await.unwrap();
    let repeated = validator.validate_credential("opaque-a").await.unwrap();
    let different = validator.validate_credential("opaque-b").await.unwrap();
    assert_eq!(first.token_id, repeated.token_id);
    assert_ne!(first.token_id, different.token_id);
    let fingerprint = first.token_id.as_deref().unwrap();
    assert!(fingerprint.starts_with("sha256:"));
    let diagnostic = format!("{first:?}");
    assert!(diagnostic.contains("<redacted>"));
    assert!(!diagnostic.contains(fingerprint));
    assert!(!diagnostic.contains("opaque-a"));
}

#[tokio::test]
async fn production_policy_can_require_provider_token_id() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/introspect"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "active": true,
            "sub": "id_alice"
        })))
        .mount(&server)
        .await;
    let validator =
        OAuth2IntrospectionValidator::new(format!("{}/introspect", server.uri()).parse().unwrap())
            .with_required_token_id();

    assert!(matches!(
        validator.validate_credential("opaque-token").await,
        Err(BearerAuthError::Invalid(ref reason)) if reason.contains("required token id")
    ));
}

#[tokio::test]
async fn introspection_metadata_is_validated_before_exposure() {
    let server = MockServer::start().await;
    Mock::given(method("POST"))
        .and(path("/introspect"))
        .respond_with(ResponseTemplate::new(200).set_body_json(json!({
            "active": true,
            "sub": "id_alice",
            "jti": "",
            "iat": u64::MAX
        })))
        .mount(&server)
        .await;
    let validator =
        OAuth2IntrospectionValidator::new(format!("{}/introspect", server.uri()).parse().unwrap());

    assert!(matches!(
        validator.validate_credential("opaque-token").await,
        Err(BearerAuthError::Invalid(ref reason)) if reason.contains("token id")
    ));
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
