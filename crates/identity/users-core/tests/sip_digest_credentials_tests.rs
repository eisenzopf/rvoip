use tempfile::TempDir;
use users_core::config::{PasswordConfig, TlsSettings};
use users_core::jwt::JwtConfig;
use users_core::{
    init, CreateSipDigestCredentialRequest, CreateUserRequest, SipDigestAlgorithmFamily,
    UsersConfig,
};

fn create_test_config(db_url: String) -> UsersConfig {
    UsersConfig {
        database_url: db_url,
        jwt: JwtConfig {
            issuer: "https://test.rvoip.local".to_string(),
            audience: vec!["rvoip-api".to_string(), "rvoip-sip".to_string()],
            access_ttl_seconds: 300,
            refresh_ttl_seconds: 3600,
            algorithm: "HS256".to_string(),
            tenant_id: None,
            signing_key: Some("test-secret-key-for-users-core".to_string()),
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

async fn setup() -> (TempDir, users_core::AuthenticationService) {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("users.db");
    let db_url = format!("sqlite://{}?mode=rwc", db_path.display());
    let service = init(create_test_config(db_url)).await.unwrap();
    service
        .create_user(CreateUserRequest {
            username: "alice".to_string(),
            password: "SecurePass2024".to_string(),
            email: Some("alice@example.com".to_string()),
            display_name: Some("Alice".to_string()),
            roles: vec!["user".to_string()],
        })
        .await
        .unwrap();
    (temp_dir, service)
}

#[tokio::test]
async fn verify_password_only_checks_credentials_without_token_issuance() {
    let (_temp_dir, service) = setup().await;

    let user = service
        .verify_password_only("alice", "SecurePass2024")
        .await
        .unwrap();
    assert_eq!(user.username, "alice");

    let wrong = service.verify_password_only("alice", "WrongPass2024").await;
    assert!(matches!(wrong, Err(users_core::Error::InvalidCredentials)));
}

#[tokio::test]
async fn sip_digest_credentials_create_rotate_lookup_and_delete() {
    let (_temp_dir, service) = setup().await;
    let user = service
        .verify_password_only("alice", "SecurePass2024")
        .await
        .unwrap();

    let created = service
        .create_sip_digest_credential(CreateSipDigestCredentialRequest {
            user_id: user.id.clone(),
            sip_username: "1001".to_string(),
            realm: "pbx.example.com".to_string(),
            algorithm: SipDigestAlgorithmFamily::Sha256,
            password: "sip-secret-one".to_string(),
        })
        .await
        .unwrap();
    assert_eq!(created.sip_username, "1001");
    assert!(!created.ha1.is_empty());

    let found = service
        .lookup_sip_digest_credential("1001", "pbx.example.com", SipDigestAlgorithmFamily::Sha256)
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.ha1, created.ha1);

    let rotated = service
        .rotate_sip_digest_credential(
            user.id,
            "1001",
            "pbx.example.com",
            SipDigestAlgorithmFamily::Sha256,
            "sip-secret-two",
        )
        .await
        .unwrap();
    assert_ne!(rotated.ha1, created.ha1);

    service
        .delete_sip_digest_credential("1001", "pbx.example.com", SipDigestAlgorithmFamily::Sha256)
        .await
        .unwrap();
    let missing = service
        .lookup_sip_digest_credential("1001", "pbx.example.com", SipDigestAlgorithmFamily::Sha256)
        .await
        .unwrap();
    assert!(missing.is_none());
}
