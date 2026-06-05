use std::sync::Arc;

use rvoip_auth_core::{
    ApiKeyVerifier, BearerAuthError, BearerValidator, CredentialAuthError, DigestAlgorithm,
    DigestSecret, DigestSecretProvider, PasswordVerifier,
};
use rvoip_core_traits::identity::IdentityAssurance;
use tempfile::TempDir;
use users_core::api_keys::CreateApiKeyRequest;
use users_core::config::{PasswordConfig, TlsSettings};
use users_core::jwt::JwtConfig;
use users_core::{
    init, CreateSipDigestCredentialRequest, CreateUserRequest, SipDigestAlgorithmFamily,
    UpdateUserRequest, UsersConfig, UsersCoreAuthProvider,
};

fn create_test_config(db_url: String) -> UsersConfig {
    UsersConfig {
        database_url: db_url,
        jwt: JwtConfig {
            issuer: "https://users.rvoip.local".to_string(),
            audience: vec!["rvoip-sip".to_string()],
            access_ttl_seconds: 300,
            refresh_ttl_seconds: 3600,
            algorithm: "HS256".to_string(),
            signing_key: Some("bridge-test-secret".to_string()),
        },
        password: PasswordConfig {
            min_length: 12,
            require_uppercase: true,
            require_lowercase: true,
            require_numbers: true,
            require_special: false,
            argon2_memory_cost: 1024,
            argon2_time_cost: 2,
            argon2_parallelism: 1,
        },
        api_bind_address: "127.0.0.1:0".to_string(),
        tls: TlsSettings::default(),
    }
}

#[tokio::test]
async fn users_core_provider_implements_auth_core_traits() {
    let seeded = seed_users_core().await;
    let provider = UsersCoreAuthProvider::shared(Arc::new(seeded.service));

    let bearer = BearerValidator::validate(provider.as_ref(), &seeded.access_token)
        .await
        .unwrap();
    assert_user_authorized(bearer);

    provider
        .auth_service()
        .revoke_access_token(&seeded.access_token)
        .await
        .unwrap();
    let revoked = BearerValidator::validate(provider.as_ref(), &seeded.access_token).await;
    assert!(
        matches!(revoked, Err(BearerAuthError::Invalid(ref err)) if err.contains("revoked")),
        "revoked users-core access tokens must fail Bearer validation: {revoked:?}"
    );

    let password = PasswordVerifier::verify_password(provider.as_ref(), "alice", "SecurePass2024")
        .await
        .unwrap();
    assert_user_authorized(password);

    let api_key = ApiKeyVerifier::verify_api_key(provider.as_ref(), &seeded.raw_api_key)
        .await
        .unwrap();
    assert_user_authorized(api_key);

    let digest = DigestSecretProvider::lookup_digest_secret(
        provider.as_ref(),
        "1001",
        "pbx.example.com",
        DigestAlgorithm::SHA256Sess,
    )
    .await
    .unwrap()
    .unwrap();
    assert!(matches!(digest, DigestSecret::Ha1(ha1) if !ha1.is_empty()));
}

#[tokio::test]
async fn users_core_provider_rejects_inactive_users_across_auth_traits() {
    let seeded = seed_users_core().await;
    seeded
        .service
        .user_store()
        .update_user(
            &seeded.user.id,
            UpdateUserRequest {
                email: None,
                display_name: None,
                roles: None,
                active: Some(false),
            },
        )
        .await
        .unwrap();
    let provider = UsersCoreAuthProvider::shared(Arc::new(seeded.service));

    let bearer = BearerValidator::validate(provider.as_ref(), &seeded.access_token).await;
    assert!(
        matches!(bearer, Err(BearerAuthError::Invalid(ref err)) if err.contains("inactive")),
        "access tokens for inactive users must fail Bearer validation: {bearer:?}"
    );

    let password =
        PasswordVerifier::verify_password(provider.as_ref(), "alice", "SecurePass2024").await;
    assert!(
        matches!(password, Err(CredentialAuthError::Invalid)),
        "inactive users must fail password verification: {password:?}"
    );

    let api_key = ApiKeyVerifier::verify_api_key(provider.as_ref(), &seeded.raw_api_key).await;
    assert!(
        matches!(api_key, Err(CredentialAuthError::Invalid)),
        "API keys for inactive users must fail verification: {api_key:?}"
    );
}

#[tokio::test]
async fn users_core_provider_rejects_revoked_api_keys() {
    let seeded = seed_users_core().await;
    seeded
        .service
        .api_key_store()
        .revoke_api_key(&seeded.api_key_id)
        .await
        .unwrap();
    let provider = UsersCoreAuthProvider::shared(Arc::new(seeded.service));

    let api_key = ApiKeyVerifier::verify_api_key(provider.as_ref(), &seeded.raw_api_key).await;
    assert!(
        matches!(api_key, Err(CredentialAuthError::Invalid)),
        "revoked API keys must fail verification: {api_key:?}"
    );
}

#[tokio::test]
async fn users_core_provider_rejects_disabled_api_keys() {
    let seeded = seed_users_core().await;
    seeded
        .service
        .api_key_store()
        .set_api_key_active(&seeded.api_key_id, false)
        .await
        .unwrap();
    let provider = UsersCoreAuthProvider::shared(Arc::new(seeded.service));

    let api_key = ApiKeyVerifier::verify_api_key(provider.as_ref(), &seeded.raw_api_key).await;
    assert!(
        matches!(api_key, Err(CredentialAuthError::Invalid)),
        "disabled API keys must fail verification: {api_key:?}"
    );
}

#[tokio::test]
async fn users_core_provider_observes_digest_rotation_and_deletion() {
    let seeded = seed_users_core().await;
    let user_id = seeded.user.id.clone();
    let provider = UsersCoreAuthProvider::shared(Arc::new(seeded.service));

    let old = DigestSecretProvider::lookup_digest_secret(
        provider.as_ref(),
        "1001",
        "pbx.example.com",
        DigestAlgorithm::SHA256,
    )
    .await
    .unwrap()
    .unwrap();

    provider
        .auth_service()
        .rotate_sip_digest_credential(
            user_id,
            "1001",
            "pbx.example.com",
            SipDigestAlgorithmFamily::Sha256,
            "sip-secret-two",
        )
        .await
        .unwrap();

    let rotated = DigestSecretProvider::lookup_digest_secret(
        provider.as_ref(),
        "1001",
        "pbx.example.com",
        DigestAlgorithm::SHA256,
    )
    .await
    .unwrap()
    .unwrap();
    assert_ne!(
        digest_secret_value(&old),
        digest_secret_value(&rotated),
        "rotating SIP Digest credentials must replace HA1 material"
    );

    provider
        .auth_service()
        .delete_sip_digest_credential("1001", "pbx.example.com", SipDigestAlgorithmFamily::Sha256)
        .await
        .unwrap();
    let deleted = DigestSecretProvider::lookup_digest_secret(
        provider.as_ref(),
        "1001",
        "pbx.example.com",
        DigestAlgorithm::SHA256,
    )
    .await
    .unwrap();
    assert!(
        deleted.is_none(),
        "deleted SIP Digest credentials must no longer be returned"
    );
}

struct SeededUsersCore {
    _temp_dir: TempDir,
    service: users_core::AuthenticationService,
    user: users_core::User,
    access_token: String,
    raw_api_key: String,
    api_key_id: String,
}

async fn seed_users_core() -> SeededUsersCore {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("users.db");
    let db_url = format!("sqlite://{}?mode=rwc", db_path.display());
    let service = init(create_test_config(db_url)).await.unwrap();
    let user = service
        .create_user(CreateUserRequest {
            username: "alice".to_string(),
            password: "SecurePass2024".to_string(),
            email: Some("alice@example.com".to_string()),
            display_name: Some("Alice".to_string()),
            roles: vec!["user".to_string()],
        })
        .await
        .unwrap();
    let auth = service
        .authenticate_password("alice", "SecurePass2024")
        .await
        .unwrap();
    let (api_key, raw_api_key) = service
        .api_key_store()
        .create_api_key(CreateApiKeyRequest {
            user_id: user.id.clone(),
            name: "sip".to_string(),
            permissions: vec!["sip.register".to_string()],
            expires_at: None,
        })
        .await
        .unwrap();
    service
        .create_sip_digest_credential(CreateSipDigestCredentialRequest {
            user_id: user.id.clone(),
            sip_username: "1001".to_string(),
            realm: "pbx.example.com".to_string(),
            algorithm: SipDigestAlgorithmFamily::Sha256,
            password: "sip-secret".to_string(),
        })
        .await
        .unwrap();
    SeededUsersCore {
        _temp_dir: temp_dir,
        service,
        user,
        access_token: auth.access_token,
        raw_api_key,
        api_key_id: api_key.id,
    }
}

fn assert_user_authorized(assurance: IdentityAssurance) {
    match assurance {
        IdentityAssurance::UserAuthorized { scopes, .. } => {
            assert!(!scopes.is_empty());
        }
        other => panic!("expected UserAuthorized, got {other:?}"),
    }
}

fn digest_secret_value(secret: &DigestSecret) -> &str {
    match secret {
        DigestSecret::Ha1(value) | DigestSecret::PlaintextPassword(value) => value,
    }
}
