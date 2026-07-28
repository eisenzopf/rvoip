//! Integration tests for [`rvoip_auth_core::JwtValidator`].
//!
//! Mints tokens with `jsonwebtoken::encode` and validates them through
//! the trait surface so the test exercises the same code path the UCTP
//! coordinator drives in production (plan A1 / G1 → C4 prelude).

use chrono::Utc;
use jsonwebtoken::{encode, Algorithm, DecodingKey, EncodingKey, Header};
use rvoip_auth_core::{
    AuthenticationMethod, BearerAuthError, BearerValidator, CredentialAuthError, JwtValidator,
    TokenRevocationChecker, TokenRevocationContext, TokenRevocationStatus,
    MAX_BEARER_TOKEN_ID_BYTES,
};
use rvoip_core_traits::identity::IdentityAssurance;
use serde::Serialize;
use std::sync::Arc;
use std::time::{Duration, UNIX_EPOCH};

const HMAC_SECRET: &[u8] = b"test-secret-key-do-not-use-in-prod";

#[derive(Serialize)]
struct TestClaims {
    sub: String,
    exp: i64,
    #[serde(skip_serializing_if = "Option::is_none")]
    jti: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    iat: Option<u64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    aud: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    iss: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    scope: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    scopes: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    roles: Option<Vec<String>>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tenant_id: Option<String>,
}

impl Default for TestClaims {
    fn default() -> Self {
        Self {
            sub: "id_alice".into(),
            exp: (Utc::now() + chrono::Duration::hours(1)).timestamp(),
            jti: None,
            iat: None,
            aud: None,
            iss: None,
            scope: None,
            scopes: None,
            roles: None,
            tenant_id: None,
        }
    }
}

struct StaticRevocationChecker {
    revoked: &'static str,
}

#[async_trait::async_trait]
impl TokenRevocationChecker for StaticRevocationChecker {
    async fn check_token(
        &self,
        context: &TokenRevocationContext,
    ) -> Result<TokenRevocationStatus, CredentialAuthError> {
        if context.token_id == self.revoked {
            Ok(TokenRevocationStatus::Revoked)
        } else {
            Ok(TokenRevocationStatus::Active)
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
async fn principal_preserves_jwt_authorization_claims() {
    let validator = JwtValidator::from_hmac_secret(HMAC_SECRET);
    let expires_at = (Utc::now() + chrono::Duration::hours(1)).timestamp();
    let token = mint(&TestClaims {
        sub: "shared-subject".into(),
        exp: expires_at,
        iss: Some("https://issuer.example".into()),
        scope: Some("calls:read calls:write".into()),
        tenant_id: Some("tenant-a".into()),
        ..Default::default()
    });

    let principal = validator.validate_principal(&token).await.unwrap();

    assert_eq!(principal.subject, "shared-subject");
    assert_eq!(principal.tenant.as_deref(), Some("tenant-a"));
    assert_eq!(principal.issuer.as_deref(), Some("https://issuer.example"));
    assert_eq!(principal.scopes, vec!["calls:read", "calls:write"]);
    assert_eq!(principal.method, AuthenticationMethod::Jwt);
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
async fn credential_preserves_validated_jti_and_iat() {
    let validator = JwtValidator::from_hmac_secret(HMAC_SECRET);
    let issued_at = u64::try_from(Utc::now().timestamp()).unwrap() - 30;
    let token = mint(&TestClaims {
        jti: Some("token-id-123".into()),
        iat: Some(issued_at),
        ..Default::default()
    });

    let credential = validator.validate_credential(&token).await.unwrap();
    assert_eq!(credential.token_id.as_deref(), Some("token-id-123"));
    assert_eq!(
        credential.issued_at,
        Some(UNIX_EPOCH + Duration::from_secs(issued_at))
    );
    assert_eq!(credential.principal.subject, "id_alice");
}

#[tokio::test]
async fn production_jti_policy_and_metadata_bounds_fail_closed() {
    let required = JwtValidator::from_hmac_secret(HMAC_SECRET).with_required_jti();
    assert!(matches!(
        required
            .validate_credential(&mint(&TestClaims::default()))
            .await,
        Err(BearerAuthError::Invalid(ref reason)) if reason.contains("required jti")
    ));

    let validator = JwtValidator::from_hmac_secret(HMAC_SECRET);
    for jti in [
        "".to_string(),
        "line\nbreak".to_string(),
        "x".repeat(MAX_BEARER_TOKEN_ID_BYTES + 1),
    ] {
        let token = mint(&TestClaims {
            jti: Some(jti),
            ..Default::default()
        });
        assert!(matches!(
            validator.validate_credential(&token).await,
            Err(BearerAuthError::Invalid(ref reason)) if reason.contains("token id")
        ));
    }
}

#[tokio::test]
async fn issued_at_range_and_order_fail_closed() {
    let validator = JwtValidator::from_hmac_secret(HMAC_SECRET);
    let expires_at = (Utc::now() + chrono::Duration::hours(1)).timestamp();
    let after_expiry = u64::try_from(expires_at).unwrap() + 1;
    let token = mint(&TestClaims {
        exp: expires_at,
        iat: Some(after_expiry),
        ..Default::default()
    });
    assert!(matches!(
        validator.validate_credential(&token).await,
        Err(BearerAuthError::Invalid(ref reason)) if reason.contains("issued-at") && reason.contains("expiry")
    ));

    let token = mint(&TestClaims {
        iat: Some(u64::MAX),
        ..Default::default()
    });
    assert!(matches!(
        validator.validate_credential(&token).await,
        Err(BearerAuthError::Invalid(ref reason)) if reason.contains("iat") && reason.contains("range")
    ));
}

#[tokio::test]
async fn validator_from_decoding_key_validates_hmac_token() {
    let validator =
        JwtValidator::from_decoding_key(DecodingKey::from_secret(HMAC_SECRET), Algorithm::HS256);
    let token = mint(&TestClaims::default());

    let assurance = validator
        .validate(&token)
        .await
        .expect("valid token must yield Ok");

    match assurance {
        IdentityAssurance::UserAuthorized { user_id, .. } => {
            assert_eq!(user_id.as_str(), "id_alice");
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
async fn token_with_unexpected_algorithm_rejects() {
    let validator = JwtValidator::from_hmac_secret(HMAC_SECRET);
    let claims = TestClaims::default();
    let token = encode(
        &Header::new(Algorithm::HS384),
        &claims,
        &EncodingKey::from_secret(HMAC_SECRET),
    )
    .expect("encode HS384 token");

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
async fn principal_boundary_rejects_token_inside_decoder_leeway() {
    let validator = JwtValidator::from_hmac_secret(HMAC_SECRET);
    let token = mint(&TestClaims {
        // jsonwebtoken accepts this inside its default 60-second clock-skew
        // leeway; the principal boundary must still reject it as inactive.
        exp: (Utc::now() - chrono::Duration::seconds(1)).timestamp(),
        ..Default::default()
    });

    let result = validator.validate_principal(&token).await;
    assert!(
        matches!(result, Err(BearerAuthError::Invalid(ref reason)) if reason.contains("expired"))
    );
}

#[tokio::test]
async fn audience_constraint_enforced() {
    let validator = JwtValidator::from_hmac_secret(HMAC_SECRET).with_audience(["uctp.example.com"]);

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
    let validator =
        JwtValidator::from_hmac_secret(HMAC_SECRET).with_issuer(["https://idp.example.com"]);

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

#[tokio::test]
async fn top_level_roles_map_to_role_scopes() {
    let validator = JwtValidator::from_hmac_secret(HMAC_SECRET);
    let claims = TestClaims {
        roles: Some(vec!["admin".into(), "operator".into()]),
        ..Default::default()
    };
    let token = mint(&claims);
    let assurance = validator.validate(&token).await.unwrap();
    if let IdentityAssurance::UserAuthorized { scopes, .. } = assurance {
        assert!(scopes.contains(&"role:admin".to_string()));
        assert!(scopes.contains(&"role:operator".to_string()));
    } else {
        panic!("expected UserAuthorized");
    }
}

#[tokio::test]
async fn revocation_checker_accepts_active_jti() {
    let validator = JwtValidator::from_hmac_secret(HMAC_SECRET).with_revocation_checker(Arc::new(
        StaticRevocationChecker {
            revoked: "revoked-token",
        },
    ));
    let token = mint(&TestClaims {
        jti: Some("active-token".into()),
        ..Default::default()
    });

    assert!(validator.validate(&token).await.is_ok());
}

#[tokio::test]
async fn revocation_checker_rejects_revoked_jti() {
    let validator = JwtValidator::from_hmac_secret(HMAC_SECRET).with_revocation_checker(Arc::new(
        StaticRevocationChecker {
            revoked: "revoked-token",
        },
    ));
    let token = mint(&TestClaims {
        jti: Some("revoked-token".into()),
        ..Default::default()
    });

    let result = validator.validate(&token).await;
    assert!(matches!(result, Err(BearerAuthError::Invalid(err)) if err.contains("revoked")));
}

#[tokio::test]
async fn revocation_checker_rejects_missing_jti() {
    let validator = JwtValidator::from_hmac_secret(HMAC_SECRET).with_revocation_checker(Arc::new(
        StaticRevocationChecker {
            revoked: "revoked-token",
        },
    ));
    let token = mint(&TestClaims::default());

    let result = validator.validate(&token).await;
    assert!(matches!(result, Err(BearerAuthError::Invalid(err)) if err.contains("missing jti")));
}
