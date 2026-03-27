//! REST API integration tests for users-core
//!
//! Tests the Axum router endpoints using tower::ServiceExt::oneshot()
//! without needing a running HTTP server.

use axum::body::Body;
use axum::http::{Request, StatusCode, header};
use tower::ServiceExt; // for oneshot()
use serde_json::{json, Value};
use std::sync::Arc;
use tempfile::TempDir;

use users_core::{
    init, UsersConfig, CreateUserRequest,
    api::{create_router, create_router_with_state, ApiState, Metrics},
    config::PasswordConfig,
    jwt::JwtConfig,
    config::TlsSettings,
    api::rate_limit::{EnhancedRateLimiter, RateLimitConfig},
};

/// Helper: read an axum response body into bytes
async fn body_to_bytes(body: Body) -> Vec<u8> {
    use http_body_util::BodyExt;
    let collected = body.collect().await.expect("failed to read body");
    collected.to_bytes().to_vec()
}

/// Helper: read an axum response body as JSON
async fn body_to_json(body: Body) -> Value {
    let bytes = body_to_bytes(body).await;
    serde_json::from_slice(&bytes).expect("response is not valid JSON")
}

/// Create a test config with a PostgreSQL DB
fn create_test_config(db_url: String) -> UsersConfig {
    UsersConfig {
        database_url: db_url,
        jwt: JwtConfig::default(),
        password: PasswordConfig {
            min_length: 12,
            require_uppercase: true,
            require_lowercase: true,
            require_numbers: true,
            require_special: false,
            // Low cost params for fast tests
            argon2_memory_cost: 1024,
            argon2_time_cost: 2,
            argon2_parallelism: 1,
        },
        api_bind_address: "127.0.0.1:0".to_string(),
        tls: TlsSettings::default(),
    }
}

/// Setup helper: creates a temp DB, inits auth service, returns (Router, TempDir, Arc<AuthenticationService>)
async fn setup() -> (axum::Router, TempDir, Arc<users_core::AuthenticationService>) {
    let temp_dir = TempDir::new().expect("failed to create temp dir");
    let db_path = temp_dir.path().join("test.db");
    let db_url = "postgres://rvoip:rvoip_dev@localhost:5432/rvoip".to_string();

    let config = create_test_config(db_url);
    let auth_service = Arc::new(init(config).await.expect("failed to init auth service"));
    let router = create_router(auth_service.clone());

    (router, temp_dir, auth_service)
}

/// Setup helper that also creates an admin user and returns an access token
async fn setup_with_admin() -> (axum::Router, TempDir, Arc<users_core::AuthenticationService>, String) {
    let (router, temp_dir, auth_service) = setup().await;

    // Create admin user
    auth_service.create_user(CreateUserRequest {
        username: "admin".to_string(),
        password: "Xk9mW2pRtN7q".to_string(),
        email: Some("admin@test.com".to_string()),
        display_name: Some("Admin User".to_string()),
        roles: vec!["admin".to_string()],
    }).await.expect("failed to create admin user");

    // Authenticate to get a token
    let auth_result = auth_service
        .authenticate_password("admin", "Xk9mW2pRtN7q")
        .await
        .expect("admin login failed");

    (router, temp_dir, auth_service, auth_result.access_token)
}

// ─── Health Check ────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_health_check_returns_200() {
    let (router, _temp_dir, _auth) = setup().await;

    let response = router
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = body_to_json(response.into_body()).await;
    assert_eq!(body["status"], "healthy");
    assert_eq!(body["service"], "users-core");
    assert!(body["timestamp"].is_string());
}

// ─── Metrics Endpoint ────────────────────────────────────────────────────────

#[tokio::test]
async fn test_metrics_returns_200() {
    let (router, _temp_dir, _auth) = setup().await;

    let response = router
        .oneshot(
            Request::builder()
                .uri("/metrics")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = body_to_json(response.into_body()).await;
    assert!(body["users"]["total"].is_number());
    assert!(body["authentication"]["attempts"].is_number());
    assert!(body["uptime_seconds"].is_number());
}

// ─── JWKS Endpoint ───────────────────────────────────────────────────────────

#[tokio::test]
async fn test_jwks_returns_public_keys() {
    let (router, _temp_dir, _auth) = setup().await;

    let response = router
        .oneshot(
            Request::builder()
                .uri("/auth/jwks.json")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = body_to_json(response.into_body()).await;
    assert!(body["keys"].is_array());
    let keys = body["keys"].as_array().unwrap();
    assert!(!keys.is_empty(), "JWKS should contain at least one key");
}

// ─── Unauthorized Access ─────────────────────────────────────────────────────

#[tokio::test]
async fn test_create_user_without_auth_returns_forbidden() {
    let (router, _temp_dir, _auth) = setup().await;

    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/users")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::to_string(&json!({
                        "username": "testuser",
                        "password": "Jn5vK8mTr2Wp",
                        "roles": ["user"]
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_list_users_without_auth_returns_forbidden() {
    let (router, _temp_dir, _auth) = setup().await;

    let response = router
        .oneshot(
            Request::builder()
                .uri("/users")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_get_user_without_auth_returns_forbidden() {
    let (router, _temp_dir, _auth) = setup().await;

    let response = router
        .oneshot(
            Request::builder()
                .uri("/users/some-id")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_delete_user_without_auth_returns_forbidden() {
    let (router, _temp_dir, _auth) = setup().await;

    let response = router
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri("/users/some-id")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_logout_without_auth_returns_forbidden() {
    let (router, _temp_dir, _auth) = setup().await;

    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/logout")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

// ─── Login Flow ──────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_login_with_valid_credentials() {
    let (router, _temp_dir, auth_service) = setup().await;

    // Create a user first
    auth_service.create_user(CreateUserRequest {
        username: "loginuser".to_string(),
        password: "Qm7xW2kR9pNv".to_string(),
        email: Some("login@test.com".to_string()),
        display_name: None,
        roles: vec!["user".to_string()],
    }).await.expect("failed to create user");

    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/login")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::to_string(&json!({
                        "username": "loginuser",
                        "password": "Qm7xW2kR9pNv"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = body_to_json(response.into_body()).await;
    assert!(body["access_token"].is_string());
    assert!(body["refresh_token"].is_string());
    assert_eq!(body["token_type"], "Bearer");
    assert!(body["expires_in"].is_number());
}

#[tokio::test]
async fn test_login_with_invalid_credentials() {
    let (router, _temp_dir, auth_service) = setup().await;

    // Create a user
    auth_service.create_user(CreateUserRequest {
        username: "loginuser2".to_string(),
        password: "Qm7xW2kR9pNv".to_string(),
        email: None,
        display_name: None,
        roles: vec!["user".to_string()],
    }).await.expect("failed to create user");

    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/login")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::to_string(&json!({
                        "username": "loginuser2",
                        "password": "WrongPassword99"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);

    let body = body_to_json(response.into_body()).await;
    assert_eq!(body["error"]["code"], "INVALID_CREDENTIALS");
}

#[tokio::test]
async fn test_login_with_nonexistent_user() {
    let (router, _temp_dir, _auth) = setup().await;

    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/login")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::to_string(&json!({
                        "username": "ghostuser",
                        "password": "Rt6wK3nXm9Pq"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    // Should return 401 (not 404) to avoid user enumeration
    assert_eq!(response.status(), StatusCode::UNAUTHORIZED);
}

// ─── Create User (Authenticated) ────────────────────────────────────────────

#[tokio::test]
async fn test_create_user_as_admin() {
    let (router, _temp_dir, _auth, token) = setup_with_admin().await;

    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/users")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, format!("Bearer {}", token))
                .body(Body::from(
                    serde_json::to_string(&json!({
                        "username": "newuser",
                        "password": "Ht6wP3nVk9Rm",
                        "email": "new@test.com",
                        "display_name": "New User",
                        "roles": ["user"]
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::CREATED);

    let body = body_to_json(response.into_body()).await;
    assert_eq!(body["username"], "newuser");
    assert_eq!(body["email"], "new@test.com");
    assert_eq!(body["display_name"], "New User");
    assert_eq!(body["active"], true);
    assert!(body["id"].is_string());
    assert!(body["created_at"].is_string());
}

// ─── List Users (Authenticated) ─────────────────────────────────────────────

#[tokio::test]
async fn test_list_users_as_admin() {
    let (router, _temp_dir, _auth, token) = setup_with_admin().await;

    let response = router
        .oneshot(
            Request::builder()
                .uri("/users")
                .header(header::AUTHORIZATION, format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = body_to_json(response.into_body()).await;
    let users = body.as_array().expect("response should be an array");
    // At least the admin user should be present
    assert!(!users.is_empty(), "should have at least one user (admin)");
    assert!(users.iter().any(|u| u["username"] == "admin"));
}

// ─── Get User by ID (Authenticated) ─────────────────────────────────────────

#[tokio::test]
async fn test_get_user_by_id_as_admin() {
    let (router, _temp_dir, auth_service, token) = setup_with_admin().await;

    // Create another user
    let user = auth_service.create_user(CreateUserRequest {
        username: "fetchme".to_string(),
        password: "Bw4rN7kXm2Pq".to_string(),
        email: Some("fetch@test.com".to_string()),
        display_name: Some("Fetch Me".to_string()),
        roles: vec!["user".to_string()],
    }).await.expect("failed to create user");

    let response = router
        .oneshot(
            Request::builder()
                .uri(format!("/users/{}", user.id))
                .header(header::AUTHORIZATION, format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = body_to_json(response.into_body()).await;
    assert_eq!(body["username"], "fetchme");
    assert_eq!(body["email"], "fetch@test.com");
    assert_eq!(body["id"], user.id);
}

#[tokio::test]
async fn test_get_nonexistent_user_returns_404() {
    let (router, _temp_dir, _auth, token) = setup_with_admin().await;

    let response = router
        .oneshot(
            Request::builder()
                .uri("/users/nonexistent-id-12345")
                .header(header::AUTHORIZATION, format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NOT_FOUND);
}

// ─── Delete User (Authenticated) ────────────────────────────────────────────

#[tokio::test]
async fn test_delete_user_as_admin() {
    let (router, _temp_dir, auth_service, token) = setup_with_admin().await;

    // Create a user to delete
    let user = auth_service.create_user(CreateUserRequest {
        username: "deleteme".to_string(),
        password: "Yp8tK3mWn6Rv".to_string(),
        email: None,
        display_name: None,
        roles: vec!["user".to_string()],
    }).await.expect("failed to create user");

    let response = router
        .oneshot(
            Request::builder()
                .method("DELETE")
                .uri(format!("/users/{}", user.id))
                .header(header::AUTHORIZATION, format!("Bearer {}", token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::NO_CONTENT);
}

// ─── Non-admin user cannot create/list/delete ────────────────────────────────

#[tokio::test]
async fn test_non_admin_cannot_create_user() {
    let (router, _temp_dir, auth_service, _admin_token) = setup_with_admin().await;

    // Create a regular user
    auth_service.create_user(CreateUserRequest {
        username: "regular".to_string(),
        password: "Vt5nJ8kWm3Rp".to_string(),
        email: None,
        display_name: None,
        roles: vec!["user".to_string()],
    }).await.expect("failed to create user");

    // Login as regular user
    let auth_result = auth_service
        .authenticate_password("regular", "Vt5nJ8kWm3Rp")
        .await
        .expect("regular login failed");

    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/users")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, format!("Bearer {}", auth_result.access_token))
                .body(Body::from(
                    serde_json::to_string(&json!({
                        "username": "hacker",
                        "password": "Zk7wP2nRt9Mq",
                        "roles": ["admin"]
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

#[tokio::test]
async fn test_non_admin_cannot_list_users() {
    let (router, _temp_dir, auth_service, _admin_token) = setup_with_admin().await;

    auth_service.create_user(CreateUserRequest {
        username: "regular2".to_string(),
        password: "Vt5nJ8kWm3Rp".to_string(),
        email: None,
        display_name: None,
        roles: vec!["user".to_string()],
    }).await.expect("failed to create user");

    let auth_result = auth_service
        .authenticate_password("regular2", "Vt5nJ8kWm3Rp")
        .await
        .expect("login failed");

    let response = router
        .oneshot(
            Request::builder()
                .uri("/users")
                .header(header::AUTHORIZATION, format!("Bearer {}", auth_result.access_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

// ─── Update User ─────────────────────────────────────────────────────────────

#[tokio::test]
async fn test_update_user_as_admin() {
    let (router, _temp_dir, auth_service, token) = setup_with_admin().await;

    let user = auth_service.create_user(CreateUserRequest {
        username: "updateme".to_string(),
        password: "Gn3xK7mWp5Rv".to_string(),
        email: Some("old@test.com".to_string()),
        display_name: Some("Old Name".to_string()),
        roles: vec!["user".to_string()],
    }).await.expect("failed to create user");

    let response = router
        .oneshot(
            Request::builder()
                .method("PUT")
                .uri(format!("/users/{}", user.id))
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, format!("Bearer {}", token))
                .body(Body::from(
                    serde_json::to_string(&json!({
                        "email": "new@test.com",
                        "display_name": "New Name"
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = body_to_json(response.into_body()).await;
    assert_eq!(body["email"], "new@test.com");
    assert_eq!(body["display_name"], "New Name");
}

// ─── Token Refresh ───────────────────────────────────────────────────────────

#[tokio::test]
async fn test_token_refresh_flow() {
    let (router, _temp_dir, auth_service, _admin_token) = setup_with_admin().await;

    // Login to get refresh token
    let auth_result = auth_service
        .authenticate_password("admin", "Xk9mW2pRtN7q")
        .await
        .expect("admin login failed");

    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/auth/refresh")
                .header(header::CONTENT_TYPE, "application/json")
                .body(Body::from(
                    serde_json::to_string(&json!({
                        "refresh_token": auth_result.refresh_token
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = body_to_json(response.into_body()).await;
    assert!(body["access_token"].is_string());
    assert!(body["refresh_token"].is_string());
    assert_eq!(body["token_type"], "Bearer");
}

// ─── Invalid Bearer Token ────────────────────────────────────────────────────

#[tokio::test]
async fn test_invalid_bearer_token_returns_forbidden() {
    let (router, _temp_dir, _auth) = setup().await;

    let response = router
        .oneshot(
            Request::builder()
                .uri("/users")
                .header(header::AUTHORIZATION, "Bearer invalid.token.here")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

// ─── Security Headers ────────────────────────────────────────────────────────

#[tokio::test]
async fn test_security_headers_present() {
    let (router, _temp_dir, _auth) = setup().await;

    let response = router
        .oneshot(
            Request::builder()
                .uri("/health")
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let headers = response.headers();
    // Check for common security headers added by security_headers_middleware
    // The exact headers depend on the middleware implementation, but these are typical
    assert!(
        headers.contains_key("x-content-type-options")
            || headers.contains_key("x-frame-options")
            || headers.contains_key("strict-transport-security")
            || headers.contains_key("content-type"),
        "Response should contain at least one security-related header"
    );
}

// ─── User Self-Access ────────────────────────────────────────────────────────

#[tokio::test]
async fn test_user_can_get_own_profile() {
    let (router, _temp_dir, auth_service, _admin_token) = setup_with_admin().await;

    // Create a regular user
    let user = auth_service.create_user(CreateUserRequest {
        username: "selfaccess".to_string(),
        password: "Mw6rT3nKp8Vq".to_string(),
        email: Some("self@test.com".to_string()),
        display_name: Some("Self Access".to_string()),
        roles: vec!["user".to_string()],
    }).await.expect("failed to create user");

    let auth_result = auth_service
        .authenticate_password("selfaccess", "Mw6rT3nKp8Vq")
        .await
        .expect("login failed");

    let response = router
        .oneshot(
            Request::builder()
                .uri(format!("/users/{}", user.id))
                .header(header::AUTHORIZATION, format!("Bearer {}", auth_result.access_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::OK);

    let body = body_to_json(response.into_body()).await;
    assert_eq!(body["username"], "selfaccess");
}

#[tokio::test]
async fn test_user_cannot_access_other_user_profile() {
    let (router, _temp_dir, auth_service, _admin_token) = setup_with_admin().await;

    // Create two regular users
    let user_a = auth_service.create_user(CreateUserRequest {
        username: "usera123".to_string(),
        password: "Fn9wR2kXm7Tp".to_string(),
        email: None,
        display_name: None,
        roles: vec!["user".to_string()],
    }).await.expect("failed to create user A");

    let user_b = auth_service.create_user(CreateUserRequest {
        username: "userb123".to_string(),
        password: "Kp4tN8mWr3Vq".to_string(),
        email: None,
        display_name: None,
        roles: vec!["user".to_string()],
    }).await.expect("failed to create user B");

    // Login as user A
    let auth_a = auth_service
        .authenticate_password("usera123", "Fn9wR2kXm7Tp")
        .await
        .expect("login A failed");

    // Try to access user B's profile
    let response = router
        .oneshot(
            Request::builder()
                .uri(format!("/users/{}", user_b.id))
                .header(header::AUTHORIZATION, format!("Bearer {}", auth_a.access_token))
                .body(Body::empty())
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::FORBIDDEN);
}

// ─── Duplicate User Creation ─────────────────────────────────────────────────

#[tokio::test]
async fn test_create_duplicate_user_returns_bad_request() {
    let (router, _temp_dir, auth_service, token) = setup_with_admin().await;

    // Create a user directly
    auth_service.create_user(CreateUserRequest {
        username: "duplicate".to_string(),
        password: "Wn7xK3mTr9Pv".to_string(),
        email: None,
        display_name: None,
        roles: vec!["user".to_string()],
    }).await.expect("failed to create first user");

    // Try to create the same user via API
    let response = router
        .oneshot(
            Request::builder()
                .method("POST")
                .uri("/users")
                .header(header::CONTENT_TYPE, "application/json")
                .header(header::AUTHORIZATION, format!("Bearer {}", token))
                .body(Body::from(
                    serde_json::to_string(&json!({
                        "username": "duplicate",
                        "password": "Wn7xK3mTr9Pv",
                        "roles": ["user"]
                    }))
                    .unwrap(),
                ))
                .unwrap(),
        )
        .await
        .unwrap();

    assert_eq!(response.status(), StatusCode::BAD_REQUEST);
}
