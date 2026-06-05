#![cfg(feature = "postgres")]

use std::sync::Arc;

use users_core::config::PasswordConfig;
use users_core::jwt::JwtConfig;
use users_core::{
    ApiKeyStore, AuthenticationService, CreateSipDigestCredentialRequest, CreateUserRequest,
    JwtIssuer, PostgresUserStore, SipDigestAlgorithmFamily, UserFilter, UserStore,
};

async fn live_store() -> Option<PostgresUserStore> {
    let url = std::env::var("RVOIP_USERS_POSTGRES_URL").ok()?;
    PostgresUserStore::new(&url).await.ok()
}

#[tokio::test]
async fn postgres_store_user_and_api_key_round_trip_when_configured() {
    let Some(store) = live_store().await else {
        return;
    };

    let suffix = chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default();
    let user = store
        .create_user(CreateUserRequest {
            username: format!("pg_alice_{suffix}"),
            email: Some(format!("pg_alice_{suffix}@example.test")),
            display_name: Some("Postgres Alice".to_string()),
            password: "hashed_password".to_string(),
            roles: vec!["user".to_string()],
        })
        .await
        .unwrap();

    assert_eq!(
        store
            .get_user_by_username(&user.username)
            .await
            .unwrap()
            .unwrap()
            .id,
        user.id
    );
    assert_eq!(
        store
            .list_users(UserFilter {
                active: Some(true),
                role: Some("user".to_string()),
                search: Some("pg_alice".to_string()),
                limit: Some(10),
                offset: None,
            })
            .await
            .unwrap()
            .iter()
            .filter(|candidate| candidate.id == user.id)
            .count(),
        1
    );

    let (api_key, raw_key) = store
        .create_api_key(users_core::api_keys::CreateApiKeyRequest {
            user_id: user.id.clone(),
            name: format!("test-key-{suffix}"),
            permissions: vec!["sip.register".to_string()],
            expires_at: None,
        })
        .await
        .unwrap();
    let validated = store.validate_api_key(&raw_key).await.unwrap().unwrap();
    assert!(validated.active);

    store.set_api_key_active(&api_key.id, false).await.unwrap();
    assert!(store.validate_api_key(&raw_key).await.unwrap().is_none());
    let listed = store.list_api_keys(&user.id).await.unwrap();
    assert!(listed.iter().any(|key| key.id == api_key.id && !key.active));

    store.delete_user(&user.id).await.unwrap();
}

#[tokio::test]
async fn postgres_auth_security_store_backs_auth_service_when_configured() {
    let Some(store) = live_store().await else {
        return;
    };

    let suffix = chrono::Utc::now().timestamp_nanos_opt().unwrap_or_default();
    let username = format!("pg_auth_{suffix}");
    let store = Arc::new(store);
    let jwt_issuer = JwtIssuer::new(JwtConfig {
        issuer: "https://postgres.users.rvoip.local".to_string(),
        audience: vec!["rvoip-sip".to_string()],
        access_ttl_seconds: 300,
        refresh_ttl_seconds: 3600,
        algorithm: "HS256".to_string(),
        signing_key: Some("postgres-auth-security-test-secret".to_string()),
    })
    .unwrap();
    let mut service = AuthenticationService::new(
        store.clone(),
        jwt_issuer,
        store.clone(),
        PasswordConfig {
            min_length: 12,
            require_uppercase: true,
            require_lowercase: true,
            require_numbers: true,
            require_special: false,
            argon2_memory_cost: 1024,
            argon2_time_cost: 2,
            argon2_parallelism: 1,
        },
    )
    .unwrap();
    service.set_auth_security_store(store.clone());

    let user = service
        .create_user(CreateUserRequest {
            username: username.clone(),
            email: Some(format!("{username}@example.test")),
            display_name: Some("Postgres Auth User".to_string()),
            password: "SecurePass2024".to_string(),
            roles: vec!["user".to_string()],
        })
        .await
        .unwrap();

    let auth = service
        .authenticate_password(&username, "SecurePass2024")
        .await
        .unwrap();
    assert_eq!(auth.user.id, user.id);
    assert!(store
        .get_user(&user.id)
        .await
        .unwrap()
        .unwrap()
        .last_login
        .is_some());

    assert!(service.refresh_token(&auth.refresh_token).await.is_ok());
    service.revoke_tokens(&user.id).await.unwrap();
    assert!(matches!(
        service.refresh_token(&auth.refresh_token).await,
        Err(users_core::Error::InvalidCredentials)
    ));

    let access_claims = service
        .jwt_issuer()
        .validate_access_token(&auth.access_token)
        .unwrap();
    service
        .revoke_access_token(&auth.access_token)
        .await
        .unwrap();
    assert!(service
        .is_access_token_revoked(&access_claims.jti)
        .await
        .unwrap());

    let digest = service
        .create_sip_digest_credential(CreateSipDigestCredentialRequest {
            user_id: user.id.clone(),
            sip_username: format!("10{suffix}"),
            realm: "pbx.example.test".to_string(),
            algorithm: SipDigestAlgorithmFamily::Sha256,
            password: "sip-secret-one".to_string(),
        })
        .await
        .unwrap();
    let found = service
        .lookup_sip_digest_credential(
            &digest.sip_username,
            &digest.realm,
            SipDigestAlgorithmFamily::Sha256,
        )
        .await
        .unwrap()
        .unwrap();
    assert_eq!(found.ha1, digest.ha1);

    let rotated = service
        .rotate_sip_digest_credential(
            &user.id,
            &digest.sip_username,
            &digest.realm,
            SipDigestAlgorithmFamily::Sha256,
            "sip-secret-two",
        )
        .await
        .unwrap();
    assert_ne!(rotated.ha1, digest.ha1);
    service
        .delete_sip_digest_credential(
            &digest.sip_username,
            &digest.realm,
            SipDigestAlgorithmFamily::Sha256,
        )
        .await
        .unwrap();
    assert!(service
        .lookup_sip_digest_credential(
            &digest.sip_username,
            &digest.realm,
            SipDigestAlgorithmFamily::Sha256,
        )
        .await
        .unwrap()
        .is_none());

    service
        .change_password(&user.id, "SecurePass2024", "SecurePass2025")
        .await
        .unwrap();
    assert!(matches!(
        service
            .verify_password_only(&username, "SecurePass2024")
            .await,
        Err(users_core::Error::InvalidCredentials)
    ));
    assert!(service
        .verify_password_only(&username, "SecurePass2025")
        .await
        .is_ok());

    store.delete_user(&user.id).await.unwrap();
}
