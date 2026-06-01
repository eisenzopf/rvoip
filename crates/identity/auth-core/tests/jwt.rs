//! Integration tests for [`rvoip_auth_core::JwtValidator`].
//!
//! Mints tokens with `jsonwebtoken::encode` and validates them through
//! the trait surface so the test exercises the same code path the UCTP
//! coordinator drives in production (plan A1 / G1 → C4 prelude).

use chrono::Utc;
use jsonwebtoken::{encode, EncodingKey, Header};
use rvoip_auth_core::{BearerAuthError, BearerValidator, JwtValidator};
use rvoip_core_traits::identity::IdentityAssurance;
use serde::Serialize;

const HMAC_SECRET: &[u8] = b"test-secret-key-do-not-use-in-prod";

#[derive(Serialize)]
struct TestClaims {
    sub: String,
    exp: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    aud: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    iss: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    scope: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    scopes: Option<Vec<String>>,
}

impl Default for TestClaims {
    fn default() -> Self {
        Self {
            sub: "id_alice".into(),
            exp: (Utc::now() + chrono::Duration::hours(1)).timestamp(),
            aud: None,
            iss: None,
            scope: None,
            scopes: None,
        }
    }
}

fn mint(claims: &TestClaims) -> String {
    encode(
        &Header::default(),
        claims,
        &EncodingKey::from_secret(HMAC_SECRET),
    )
    .expect("encode test JWT")
}

#[tokio::test]
async fn valid_hmac_token_yields_user_authorized() {
    let validator = JwtValidator::from_hmac_secret(HMAC_SECRET);
    let token = mint(&TestClaims::default());

    let assurance = validator
        .validate(&token)
        .await
        .expect("valid token must yield Ok");

    match assurance {
        IdentityAssurance::UserAuthorized {
            identity,
            user_id,
            scopes,
        } => {
            assert_eq!(identity.as_str(), "id_alice");
            // For a plain JWT (no actor/subject split) identity == user_id.
            assert_eq!(user_id.as_str(), "id_alice");
            assert!(scopes.is_empty());
        }
        other => panic!("expected UserAuthorized, got {:?}", other),
    }
}

#[tokio::test]
async fn empty_token_rejects_with_empty_error() {
    let validator = JwtValidator::from_hmac_secret(HMAC_SECRET);
    let result = validator.validate("").await;
    assert!(matches!(result, Err(BearerAuthError::Empty)));
}

#[tokio::test]
async fn malformed_token_rejects_with_invalid() {
    let validator = JwtValidator::from_hmac_secret(HMAC_SECRET);
    let result = validator.validate("this.is.not-a-real-jwt").await;
    assert!(matches!(result, Err(BearerAuthError::Invalid(_))));
}

#[tokio::test]
async fn token_signed_with_different_secret_rejects() {
    let validator = JwtValidator::from_hmac_secret(HMAC_SECRET);
    let claims = TestClaims::default();
    // Mint with a different secret — signature won't verify against
    // the validator's HMAC_SECRET.
    let token = encode(
        &Header::default(),
        &claims,
        &EncodingKey::from_secret(b"wrong-secret"),
    )
    .expect("encode");
    let result = validator.validate(&token).await;
    assert!(matches!(result, Err(BearerAuthError::Invalid(_))));
}

#[tokio::test]
async fn expired_token_rejects() {
    let validator = JwtValidator::from_hmac_secret(HMAC_SECRET);
    let claims = TestClaims {
        // Default Validation has 60s leeway; 10 minutes past covers it.
        exp: (Utc::now() - chrono::Duration::minutes(10)).timestamp(),
        ..Default::default()
    };
    let token = mint(&claims);
    let result = validator.validate(&token).await;
    assert!(
        matches!(result, Err(BearerAuthError::Invalid(_))),
        "expired token must be rejected"
    );
}

#[tokio::test]
async fn audience_constraint_enforced() {
    let validator =
        JwtValidator::from_hmac_secret(HMAC_SECRET).with_audience(["uctp.example.com"]);

    // Wrong audience → reject.
    let bad = mint(&TestClaims {
        aud: Some("other.example.com".into()),
        ..Default::default()
    });
    assert!(matches!(
        validator.validate(&bad).await,
        Err(BearerAuthError::Invalid(_))
    ));

    // Right audience → ok.
    let good = mint(&TestClaims {
        aud: Some("uctp.example.com".into()),
        ..Default::default()
    });
    assert!(validator.validate(&good).await.is_ok());
}

#[tokio::test]
async fn issuer_constraint_enforced() {
    let validator = JwtValidator::from_hmac_secret(HMAC_SECRET).with_issuer(["https://idp.example.com"]);

    let bad = mint(&TestClaims {
        iss: Some("https://imposter.example.com".into()),
        ..Default::default()
    });
    assert!(matches!(
        validator.validate(&bad).await,
        Err(BearerAuthError::Invalid(_))
    ));

    let good = mint(&TestClaims {
        iss: Some("https://idp.example.com".into()),
        ..Default::default()
    });
    assert!(validator.validate(&good).await.is_ok());
}

#[tokio::test]
async fn space_separated_scope_claim_parses() {
    let validator = JwtValidator::from_hmac_secret(HMAC_SECRET);
    let claims = TestClaims {
        scope: Some("read:calls write:messages admin".into()),
        ..Default::default()
    };
    let token = mint(&claims);
    let assurance = validator.validate(&token).await.unwrap();
    if let IdentityAssurance::UserAuthorized { scopes, .. } = assurance {
        assert_eq!(scopes, vec!["read:calls", "write:messages", "admin"]);
    } else {
        panic!("expected UserAuthorized");
    }
}

#[tokio::test]
async fn array_scopes_claim_parses() {
    let validator = JwtValidator::from_hmac_secret(HMAC_SECRET);
    let claims = TestClaims {
        scopes: Some(vec!["read:calls".into(), "write:messages".into()]),
        ..Default::default()
    };
    let token = mint(&claims);
    let assurance = validator.validate(&token).await.unwrap();
    if let IdentityAssurance::UserAuthorized { scopes, .. } = assurance {
        assert_eq!(scopes, vec!["read:calls", "write:messages"]);
    } else {
        panic!("expected UserAuthorized");
    }
}

#[tokio::test]
async fn both_scope_forms_merge_without_duplicates() {
    let validator = JwtValidator::from_hmac_secret(HMAC_SECRET);
    let claims = TestClaims {
        scope: Some("read:calls write:messages".into()),
        scopes: Some(vec!["read:calls".into(), "admin".into()]),
        ..Default::default()
    };
    let token = mint(&claims);
    let assurance = validator.validate(&token).await.unwrap();
    if let IdentityAssurance::UserAuthorized { scopes, .. } = assurance {
        // `read:calls` appears in both — dedup keeps one copy.
        assert_eq!(scopes.len(), 3);
        assert!(scopes.contains(&"read:calls".to_string()));
        assert!(scopes.contains(&"write:messages".to_string()));
        assert!(scopes.contains(&"admin".to_string()));
    } else {
        panic!("expected UserAuthorized");
    }
}
