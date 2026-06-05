use tempfile::TempDir;
use users_core::config::{PasswordConfig, TlsSettings};
use users_core::jwt::JwtConfig;
use users_core::{
    init, CreateUserRequest, TokenIssueContext, UpsertExternalIdentityRequest,
    UpsertPasskeyCredentialRequest, UsersConfig,
};

fn test_config(db_url: String) -> UsersConfig {
    UsersConfig {
        database_url: db_url,
        jwt: JwtConfig {
            issuer: "https://users.rvoip.local".to_string(),
            audience: vec!["rvoip-app".to_string()],
            access_ttl_seconds: 300,
            refresh_ttl_seconds: 3600,
            algorithm: "HS256".to_string(),
            signing_key: Some("enterprise-identity-test-secret".to_string()),
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

async fn seeded_service() -> (TempDir, users_core::AuthenticationService, users_core::User) {
    let temp = tempfile::tempdir().unwrap();
    let db_url = format!(
        "sqlite://{}?mode=rwc",
        temp.path().join("users.db").display()
    );
    let service = init(test_config(db_url)).await.unwrap();
    let user = service
        .create_user(CreateUserRequest {
            username: "alice".to_string(),
            password: "SecurePass2026".to_string(),
            email: Some("alice@example.test".to_string()),
            display_name: Some("Alice Example".to_string()),
            roles: vec!["user".to_string()],
        })
        .await
        .unwrap();
    (temp, service, user)
}

#[tokio::test]
async fn external_identity_links_are_created_updated_listed_and_deleted() {
    let (_temp, service, user) = seeded_service().await;
    let store = service
        .enterprise_identity_store()
        .expect("init should configure enterprise identity store");

    store
        .upsert_external_identity(UpsertExternalIdentityRequest {
            provider_id: "keycloak".to_string(),
            external_subject: "kc-subject-1".to_string(),
            user_id: user.id.clone(),
            email: Some("alice@example.test".to_string()),
            username: Some("alice".to_string()),
            display_name: Some("Alice".to_string()),
            groups: vec!["agent".to_string()],
            active: true,
        })
        .await
        .unwrap();

    let link = store
        .get_external_identity("keycloak", "kc-subject-1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(link.user_id, user.id);
    assert_eq!(link.groups, vec!["agent"]);

    store
        .upsert_external_identity(UpsertExternalIdentityRequest {
            groups: vec!["agent".to_string(), "supervisor".to_string()],
            display_name: Some("Alice Updated".to_string()),
            ..UpsertExternalIdentityRequest {
                provider_id: "keycloak".to_string(),
                external_subject: "kc-subject-1".to_string(),
                user_id: user.id.clone(),
                email: Some("alice@example.test".to_string()),
                username: Some("alice".to_string()),
                display_name: Some("Alice".to_string()),
                groups: vec![],
                active: true,
            }
        })
        .await
        .unwrap();

    let links = store
        .list_external_identities_for_user(&user.id)
        .await
        .unwrap();
    assert_eq!(links.len(), 1);
    assert_eq!(links[0].display_name.as_deref(), Some("Alice Updated"));
    assert_eq!(links[0].groups, vec!["agent", "supervisor"]);

    store
        .delete_external_identity("keycloak", "kc-subject-1")
        .await
        .unwrap();
    assert!(store
        .get_external_identity("keycloak", "kc-subject-1")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn passkey_credentials_are_created_updated_listed_and_deleted() {
    let (_temp, service, user) = seeded_service().await;
    let store = service
        .enterprise_identity_store()
        .expect("init should configure enterprise identity store");

    store
        .upsert_passkey_credential(UpsertPasskeyCredentialRequest {
            credential_id: "cred-1".to_string(),
            user_id: user.id.clone(),
            public_key: "{\"type\":\"test-passkey\"}".to_string(),
            sign_count: 1,
            transports: vec!["internal".to_string()],
            backup_eligible: true,
            backup_state: false,
            display_name: Some("MacBook passkey".to_string()),
        })
        .await
        .unwrap();

    let credential = store
        .get_passkey_credential("cred-1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(credential.user_id, user.id);
    assert_eq!(credential.sign_count, 1);
    assert_eq!(credential.transports, vec!["internal"]);

    store.update_passkey_usage("cred-1", 7).await.unwrap();
    let updated = store
        .get_passkey_credential("cred-1")
        .await
        .unwrap()
        .unwrap();
    assert_eq!(updated.sign_count, 7);
    assert!(updated.last_used_at.is_some());

    let credentials = store
        .list_passkey_credentials_for_user(&user.id)
        .await
        .unwrap();
    assert_eq!(credentials.len(), 1);

    store.delete_passkey_credential("cred-1").await.unwrap();
    assert!(store
        .get_passkey_credential("cred-1")
        .await
        .unwrap()
        .is_none());
}

#[tokio::test]
async fn issue_tokens_for_external_user_checks_active_state_and_updates_login() {
    let (_temp, service, user) = seeded_service().await;

    let result = service
        .issue_tokens_for_user(
            &user.id,
            TokenIssueContext::external_identity("saml", "keycloak-saml", "subject-1"),
        )
        .await
        .unwrap();
    assert_eq!(result.user.id, user.id);
    assert!(!result.access_token.is_empty());
    assert!(!result.refresh_token.is_empty());

    let refreshed = service
        .user_store()
        .get_user(&user.id)
        .await
        .unwrap()
        .unwrap();
    assert!(refreshed.last_login.is_some());

    service
        .user_store()
        .update_user(
            &user.id,
            users_core::UpdateUserRequest {
                email: None,
                display_name: None,
                roles: None,
                active: Some(false),
            },
        )
        .await
        .unwrap();
    let denied = service
        .issue_tokens_for_user(&user.id, TokenIssueContext::new("webauthn-passkey"))
        .await;
    assert!(matches!(denied, Err(users_core::Error::InvalidCredentials)));
}
