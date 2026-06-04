use std::sync::Arc;

use rvoip_auth_core::{
    ApiKeyVerifier, BearerAuthError, BearerValidator, DigestAlgorithm, DigestSecret,
    DigestSecretProvider, PasswordVerifier,
};
use rvoip_core_traits::identity::IdentityAssurance;
use tempfile::TempDir;
use users_core::api_keys::CreateApiKeyRequest;
use users_core::config::{PasswordConfig, TlsSettings};
use users_core::jwt::JwtConfig;
use users_core::{
    init, CreateSipDigestCredentialRequest, CreateUserRequest, SipDigestAlgorithmFamily,
    UsersConfig, UsersCoreAuthProvider,
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
    let (_, raw_api_key) = service
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

    let provider = UsersCoreAuthProvider::shared(Arc::new(service));

    let bearer = BearerValidator::validate(provider.as_ref(), &auth.access_token)
        .await
        .unwrap();
    assert_user_authorized(bearer);

    provider
        .auth_service()
        .revoke_access_token(&auth.access_token)
        .await
        .unwrap();
    let revoked = BearerValidator::validate(provider.as_ref(), &auth.access_token).await;
    assert!(
        matches!(revoked, Err(BearerAuthError::Invalid(ref err)) if err.contains("revoked")),
        "revoked users-core access tokens must fail Bearer validation: {revoked:?}"
    );

    let password = PasswordVerifier::verify_password(provider.as_ref(), "alice", "SecurePass2024")
        .await
        .unwrap();
    assert_user_authorized(password);

    let api_key = ApiKeyVerifier::verify_api_key(provider.as_ref(), &raw_api_key)
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

fn assert_user_authorized(assurance: IdentityAssurance) {
    match assurance {
        IdentityAssurance::UserAuthorized { scopes, .. } => {
            assert!(!scopes.is_empty());
        }
        other => panic!("expected UserAuthorized, got {other:?}"),
    }
}
