//! RS256 issuers must verify the tokens they issue.
//!
//! The fixture keys are test-only 2048-bit RSA keys generated for this suite.

use base64::{engine::general_purpose::URL_SAFE_NO_PAD, Engine as _};
use users_core::{
    jwt::{JwtConfig, JwtIssuer},
    User,
};

const PKCS8_KEY: &str = include_str!("fixtures/rs256-test-private-key-pkcs8.txt");
const PKCS1_KEY: &str = include_str!("fixtures/rs256-test-private-key-pkcs1.txt");

fn issuer(key: &str, tenant: &str) -> JwtIssuer {
    JwtIssuer::new(JwtConfig {
        issuer: "https://users.test".into(),
        audience: vec!["rvoip-api".into()],
        access_ttl_seconds: 300,
        refresh_ttl_seconds: 600,
        algorithm: "RS256".into(),
        tenant_id: Some(tenant.into()),
        signing_key: Some(key.into()),
    })
    .expect("construct RS256 issuer")
}

fn user() -> User {
    let now = chrono::Utc::now();
    User {
        id: "user-a".into(),
        username: "alice".into(),
        email: None,
        display_name: None,
        password_hash: "not-used-by-jwt-tests".into(),
        roles: vec!["user".into()],
        active: true,
        created_at: now,
        updated_at: now,
        last_login: None,
    }
}

fn assert_round_trip(key: &str) {
    let issuer = issuer(key, "tenant-a");
    let access = issuer.create_access_token(&user()).expect("issue access");
    let claims = issuer
        .validate_access_token(&access)
        .expect("issuer verifies its own RS256 access token");
    assert_eq!(claims.sub, "user-a");
    assert_eq!(claims.tenant_id.as_deref(), Some("tenant-a"));

    let refresh = issuer
        .create_refresh_token("user-a")
        .expect("issue refresh");
    assert_eq!(
        issuer
            .validate_refresh_token(&refresh)
            .expect("issuer verifies its own RS256 refresh token")
            .sub,
        "user-a"
    );

    // Tenant binding and signature checks still apply.
    let other_tenant = self::issuer(key, "tenant-b");
    assert!(other_tenant.validate_access_token(&access).is_err());
    assert!(other_tenant.validate_refresh_token(&refresh).is_err());
    let (body, signature) = access.rsplit_once('.').unwrap();
    let mut signature = URL_SAFE_NO_PAD.decode(signature).unwrap();
    signature[0] ^= 1;
    let tampered = format!("{body}.{}", URL_SAFE_NO_PAD.encode(signature));
    assert!(issuer.validate_access_token(&tampered).is_err());

    // A relying party holding only the published JWK verifies the same token.
    let jwk = issuer.public_key_jwk().expect("public JWK");
    let decoding_key = jsonwebtoken::DecodingKey::from_rsa_components(
        jwk["n"].as_str().unwrap(),
        jwk["e"].as_str().unwrap(),
    )
    .unwrap();
    let mut validation = jsonwebtoken::Validation::new(jsonwebtoken::Algorithm::RS256);
    validation.set_audience(&["rvoip-api"]);
    validation.set_issuer(&["https://users.test"]);
    assert!(jsonwebtoken::decode::<serde_json::Value>(&access, &decoding_key, &validation).is_ok());
}

#[test]
fn rs256_pkcs8_issuer_verifies_its_own_tokens() {
    assert_round_trip(PKCS8_KEY);
}

#[test]
fn rs256_pkcs1_issuer_verifies_its_own_tokens() {
    assert_round_trip(PKCS1_KEY);
}

#[test]
fn pkcs1_and_pkcs8_forms_publish_the_same_public_jwk() {
    let pkcs8 = issuer(PKCS8_KEY, "tenant-a").public_key_jwk().unwrap();
    let pkcs1 = issuer(PKCS1_KEY, "tenant-a").public_key_jwk().unwrap();
    assert_eq!(pkcs8["n"], pkcs1["n"]);
    assert_eq!(pkcs8["e"], pkcs1["e"]);
    for private in ["d", "p", "q", "dp", "dq", "qi"] {
        assert!(pkcs8.get(private).is_none());
    }
}

#[test]
fn malformed_rs256_signing_key_is_rejected_at_construction() {
    let result = JwtIssuer::new(JwtConfig {
        algorithm: "RS256".into(),
        signing_key: Some("-----BEGIN PRIVATE KEY-----\nAA==\n-----END PRIVATE KEY-----\n".into()),
        ..JwtConfig::default()
    });
    assert!(result.is_err());
}
