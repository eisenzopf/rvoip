use std::sync::Arc;

use axum::{
    body::Body,
    extract::ConnectInfo,
    http::{header, Request, StatusCode},
};
use tempfile::TempDir;
use tower::ServiceExt;
use users_core::{
    api::create_router, api_keys::CreateApiKeyRequest, AuthenticationService, CreateUserRequest,
    JwtIssuer, SqliteUserStore,
};

fn password_config() -> users_core::config::PasswordConfig {
    users_core::config::PasswordConfig {
        min_length: 12,
        require_uppercase: true,
        require_lowercase: true,
        require_numbers: true,
        require_special: false,
        argon2_memory_cost: 1024,
        argon2_time_cost: 2,
        argon2_parallelism: 1,
    }
}

async fn service_without_security_store() -> (TempDir, AuthenticationService, users_core::User) {
    let temp_dir = TempDir::new().unwrap();
    let database_url = format!(
        "sqlite://{}?mode=rwc",
        temp_dir.path().join("fail-closed.db").display()
    );
    let store = Arc::new(SqliteUserStore::new(&database_url).await.unwrap());
    let issuer = JwtIssuer::new(users_core::auth::JwtConfig {
        issuer: "https://fail-closed.users.test".into(),
        audience: vec!["rvoip-api".into()],
        access_ttl_seconds: 300,
        refresh_ttl_seconds: 3600,
        algorithm: "HS256".into(),
        tenant_id: None,
        signing_key: Some("fail-closed-test-signing-key".into()),
    })
    .unwrap();
    let service =
        AuthenticationService::new(store.clone(), issuer, store, password_config()).unwrap();
    let user = service
        .create_user(CreateUserRequest {
            username: "alice".into(),
            password: "SecurePassword2026".into(),
            email: Some("alice@example.test".into()),
            display_name: None,
            roles: vec!["user".into()],
        })
        .await
        .unwrap();
    (temp_dir, service, user)
}

async fn service_with_security_store() -> (TempDir, AuthenticationService, users_core::User) {
    let temp_dir = TempDir::new().unwrap();
    let database_url = format!(
        "sqlite://{}?mode=rwc",
        temp_dir.path().join("logout.db").display()
    );
    let mut config = users_core::UsersConfig::default();
    config.database_url = database_url;
    config.jwt = users_core::auth::JwtConfig {
        issuer: "https://logout.users.test".into(),
        audience: vec!["rvoip-api".into()],
        access_ttl_seconds: 300,
        refresh_ttl_seconds: 3600,
        algorithm: "HS256".into(),
        tenant_id: None,
        signing_key: Some("logout-test-signing-key".into()),
    };
    config.password = password_config();
    let service = users_core::init(config).await.unwrap();
    let user = service
        .create_user(CreateUserRequest {
            username: "alice".into(),
            password: "SecurePassword2026".into(),
            email: Some("alice@example.test".into()),
            display_name: None,
            roles: vec!["user".into()],
        })
        .await
        .unwrap();
    (temp_dir, service, user)
}

fn with_peer(mut request: Request<Body>) -> Request<Body> {
    request.extensions_mut().insert(ConnectInfo(
        "192.0.2.10:443".parse::<std::net::SocketAddr>().unwrap(),
    ));
    request
}

#[tokio::test]
async fn custom_router_without_security_store_rejects_login_refresh_and_bearer_access() {
    let (_temp_dir, service, user) = service_without_security_store().await;
    let access_token = service.jwt_issuer().create_access_token(&user).unwrap();
    let refresh_token = service.jwt_issuer().create_refresh_token(&user.id).unwrap();
    let app = create_router(Arc::new(service));

    let login = app
        .clone()
        .oneshot(with_peer(
            Request::builder()
                .method("POST")
                .uri("/auth/login")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({
                        "username": "alice",
                        "password": "SecurePassword2026"
                    })
                    .to_string(),
                ))
                .unwrap(),
        ))
        .await
        .unwrap();
    assert_eq!(login.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let refresh = app
        .clone()
        .oneshot(with_peer(
            Request::builder()
                .method("POST")
                .uri("/auth/refresh")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::json!({"refresh_token": refresh_token}).to_string(),
                ))
                .unwrap(),
        ))
        .await
        .unwrap();
    assert_eq!(refresh.status(), StatusCode::INTERNAL_SERVER_ERROR);

    let protected = app
        .oneshot(with_peer(
            Request::builder()
                .uri(format!("/users/{}", user.id))
                .header(header::AUTHORIZATION, format!("Bearer {access_token}"))
                .body(Body::empty())
                .unwrap(),
        ))
        .await
        .unwrap();
    assert_eq!(protected.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn bare_router_embedding_without_peer_metadata_fails_closed() {
    let (_temp_dir, service, _user) = service_without_security_store().await;
    let response = create_router(Arc::new(service))
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::SERVICE_UNAVAILABLE);
}

#[tokio::test]
async fn api_key_cannot_logout_or_revoke_a_users_token_lineage() {
    let (_temp_dir, service, user) = service_with_security_store().await;
    let (_key, raw_key) = service
        .api_key_store()
        .create_api_key(CreateApiKeyRequest {
            user_id: user.id,
            name: "logout-attempt".into(),
            permissions: vec!["*".into()],
            expires_at: None,
        })
        .await
        .unwrap();
    let service = Arc::new(service);

    let response = create_router(service.clone())
        .oneshot(with_peer(
            Request::builder()
                .method("POST")
                .uri("/auth/logout")
                .header("X-API-Key", &raw_key)
                .body(Body::empty())
                .unwrap(),
        ))
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
    assert!(service
        .api_key_store()
        .validate_api_key(&raw_key)
        .await
        .unwrap()
        .is_some());
}

#[tokio::test]
async fn custom_service_without_security_store_returns_typed_lifecycle_errors() {
    let (_temp_dir, service, user) = service_without_security_store().await;

    assert!(matches!(
        service
            .authenticate_password("alice", "SecurePassword2026")
            .await,
        Err(users_core::Error::SecurityStoreUnavailable {
            operation: "token-issuance"
        })
    ));
    assert!(matches!(
        service.revoke_tokens(&user.id).await,
        Err(users_core::Error::SecurityStoreUnavailable {
            operation: "token-revocation"
        })
    ));
    assert!(matches!(
        service
            .change_password(&user.id, "SecurePassword2026", "OtherPassword2026")
            .await,
        Err(users_core::Error::SecurityStoreUnavailable {
            operation: "password-change"
        })
    ));
}
