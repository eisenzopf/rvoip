//! Tests for authentication service
//! These tests demonstrate complete authentication workflows

use users_core::{init, UsersConfig, CreateUserRequest};
use users_core::config::{PasswordConfig, TlsSettings};
use users_core::jwt::JwtConfig;
use tempfile::TempDir;

/// Helper to create test configuration
fn create_test_config(db_url: String) -> UsersConfig {
    UsersConfig {
        database_url: db_url,
        jwt: JwtConfig {
            issuer: "https://test.rvoip.local".to_string(),
            audience: vec!["test-api".to_string()],
            access_ttl_seconds: 300,    // 5 minutes for tests
            refresh_ttl_seconds: 3600,  // 1 hour for tests
            algorithm: "RS256".to_string(),
            signing_key: None,  // Will be auto-generated
        },
        password: PasswordConfig {
            min_length: 12,  // Updated to match validation.rs
            require_uppercase: true,
            require_lowercase: true,
            require_numbers: true,
            require_special: false,
            argon2_memory_cost: 1024,  // Lower for tests
            argon2_time_cost: 2,
            argon2_parallelism: 1,
        },
        api_bind_address: "127.0.0.1:0".to_string(),
        tls: TlsSettings::default(),
    }
}

#[tokio::test]
async fn test_complete_authentication_flow() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.db");
    let db_url = format!("sqlite://{}?mode=rwc", db_path.display());
    
    let config = create_test_config(db_url);
    let auth_service = init(config).await.unwrap();
    
    // 1. Create a user
    let user = auth_service.create_user(CreateUserRequest {
        username: "testuser".to_string(),
        password: "SecurePass123".to_string(),
        email: Some("test@example.com".to_string()),
        display_name: Some("Test User".to_string()),
        roles: vec!["user".to_string()],
    }).await.unwrap();
    
    assert_eq!(user.username, "testuser");
    
    // 2. Authenticate with correct password
    let auth_result = auth_service.authenticate_password("testuser", "SecurePass123").await.unwrap();
    
    assert_eq!(auth_result.user.id, user.id);
    assert!(!auth_result.access_token.is_empty());
    assert!(!auth_result.refresh_token.is_empty());
    assert_eq!(auth_result.expires_in.as_secs(), 300);
    
    // 3. Verify tokens are JWT format
    assert!(auth_result.access_token.split('.').count() == 3);
    assert!(auth_result.refresh_token.split('.').count() == 3);
    
    // 4. Test incorrect password
    let fail_result = auth_service.authenticate_password("testuser", "WrongPassword123").await;
    assert!(fail_result.is_err());
    match fail_result.unwrap_err() {
        users_core::Error::InvalidCredentials => {},
        _ => panic!("Expected InvalidCredentials error"),
    }
    
    // 5. Test refresh token
    let token_pair = auth_service.refresh_token(&auth_result.refresh_token).await.unwrap();
    assert!(!token_pair.access_token.is_empty());
    assert_eq!(token_pair.refresh_token, auth_result.refresh_token);  // Same refresh token
    assert!(token_pair.access_token != auth_result.access_token);    // New access token
    
    // 6. Test token revocation
    auth_service.revoke_tokens(&user.id).await.unwrap();
    
    // Refresh should now fail
    let revoked_result = auth_service.refresh_token(&auth_result.refresh_token).await;
    assert!(revoked_result.is_err());
}

#[tokio::test]
async fn test_password_validation() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.db");
    let db_url = format!("sqlite://{}?mode=rwc", db_path.display());
    
    let config = create_test_config(db_url);
    let auth_service = init(config).await.unwrap();
    
    // Test various invalid passwords
    let test_cases = vec![
        ("short1", "Your password needs to be at least 12 characters long. Try using a passphrase!"),
        ("alllowercase123", "Password must contain an uppercase letter"),
        ("ALLUPPERCASE123", "Password must contain a lowercase letter"),
        ("NoNumbersHereAtAll", "Password must contain a number"),
    ];
    
    for (password, expected_error) in test_cases {
        let result = auth_service.create_user(CreateUserRequest {
            username: format!("user_{}", password),
            password: password.to_string(),
            email: None,
            display_name: None,
            roles: vec!["user".to_string()],
        }).await;
        
        assert!(result.is_err());
        match result.unwrap_err() {
            users_core::Error::InvalidPassword(msg) => {
                assert_eq!(msg, expected_error);
            },
            _ => panic!("Expected InvalidPassword error"),
        }
    }
}

#[tokio::test]
async fn test_api_key_authentication() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.db");
    let db_url = format!("sqlite://{}?mode=rwc", db_path.display());
    
    let config = create_test_config(db_url);
    let auth_service = init(config).await.unwrap();
    
    // Create a user
    let user = auth_service.create_user(CreateUserRequest {
        username: "apiuser".to_string(),
        password: "SecurePass2024!".to_string(),
        email: Some("api@example.com".to_string()),
        display_name: None,
        roles: vec!["user".to_string()],
    }).await.unwrap();
    
    // Create an API key
    let api_key_store = auth_service.api_key_store().clone();
    let (api_key, raw_key) = api_key_store.create_api_key(users_core::api_keys::CreateApiKeyRequest {
        user_id: user.id.clone(),
        name: "Test API Key".to_string(),
        permissions: vec!["read".to_string(), "write".to_string()],
        expires_at: None,
    }).await.unwrap();
    
    // Authenticate with API key
    let auth_result = auth_service.authenticate_api_key(&raw_key).await.unwrap();
    
    assert_eq!(auth_result.user.id, user.id);
    assert!(!auth_result.access_token.is_empty());
    assert_eq!(auth_result.expires_in.as_secs(), 300);  // API keys get shorter tokens
    
    // Invalid API key should fail
    let fail_result = auth_service.authenticate_api_key("rvoip_ak_live_invalid").await;
    assert!(fail_result.is_err());
}

#[tokio::test]
async fn test_change_password() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.db");
    let db_url = format!("sqlite://{}?mode=rwc", db_path.display());
    
    let config = create_test_config(db_url);
    let auth_service = init(config).await.unwrap();
    
    // Create a user
    let user = auth_service.create_user(CreateUserRequest {
        username: "changepass".to_string(),
        password: "OldPassword123".to_string(),
        email: None,
        display_name: None,
        roles: vec!["user".to_string()],
    }).await.unwrap();
    
    // Get initial tokens
    let initial_auth = auth_service.authenticate_password("changepass", "OldPassword123").await.unwrap();
    
    // Change password
    auth_service.change_password(&user.id, "OldPassword123", "NewPassword456").await.unwrap();
    
    // Old password should fail
    let old_pass_result = auth_service.authenticate_password("changepass", "OldPassword123").await;
    assert!(old_pass_result.is_err());
    
    // New password should work
    let new_auth = auth_service.authenticate_password("changepass", "NewPassword456").await.unwrap();
    assert_eq!(new_auth.user.id, user.id);
    
    // Old tokens should be revoked
    let refresh_result = auth_service.refresh_token(&initial_auth.refresh_token).await;
    assert!(refresh_result.is_err());
}

#[tokio::test]
async fn test_inactive_user_cannot_authenticate() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.db");
    let db_url = format!("sqlite://{}?mode=rwc", db_path.display());
    
    let config = create_test_config(db_url);
    let auth_service = init(config).await.unwrap();
    
    // Create a user
    let user = auth_service.create_user(CreateUserRequest {
        username: "inactiveuser".to_string(),
        password: "SecurePass2024".to_string(),
        email: None,
        display_name: None,
        roles: vec!["user".to_string()],
    }).await.unwrap();
    
    // Deactivate the user
    let user_store = auth_service.user_store().clone();
    user_store.update_user(&user.id, users_core::UpdateUserRequest {
        email: None,
        display_name: None,
        roles: None,
        active: Some(false),
    }).await.unwrap();
    
    // Authentication should fail
    let auth_result = auth_service.authenticate_password("inactiveuser", "SecurePass2024").await;
    assert!(auth_result.is_err());
    match auth_result.unwrap_err() {
        users_core::Error::InvalidCredentials => {},
        _ => panic!("Expected InvalidCredentials error"),
    }
}

#[tokio::test]
async fn test_jwt_claims_content() {
    use jsonwebtoken::{decode, Algorithm, Validation, DecodingKey};
    
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("test.db");
    let db_url = format!("sqlite://{}?mode=rwc", db_path.display());
    
    let config = create_test_config(db_url);
    let auth_service = init(config).await.unwrap();
    
    // Create a user with specific roles
    let user = auth_service.create_user(CreateUserRequest {
        username: "claimstest".to_string(),
        password: "SecurePass2024".to_string(),
        email: Some("claims@example.com".to_string()),
        display_name: Some("Claims Test".to_string()),
        roles: vec!["user".to_string(), "admin".to_string()],
    }).await.unwrap();
    
    let auth_result = auth_service.authenticate_password("claimstest", "SecurePass2024").await.unwrap();
    
    // Get public key for validation
    let public_key_pem = auth_service.jwt_issuer().public_key_pem().unwrap();
    let decoding_key = DecodingKey::from_rsa_pem(public_key_pem.as_bytes()).unwrap();
    
    // Decode and verify claims
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_issuer(&["https://test.rvoip.local"]);
    validation.set_audience(&["test-api"]);
    
    let token_data = decode::<users_core::UserClaims>(
        &auth_result.access_token,
        &decoding_key,
        &validation
    ).unwrap();
    
    let claims = token_data.claims;
    assert_eq!(claims.sub, user.id);
    assert_eq!(claims.username, "claimstest");
    assert_eq!(claims.email.as_deref(), Some("claims@example.com"));
    assert_eq!(claims.roles, vec!["user", "admin"]);
    assert!(claims.scope.contains("openid"));
    assert!(claims.scope.contains("profile"));
    assert!(claims.scope.contains("email"));
    assert!(claims.scope.contains("sip.register"));
    assert!(claims.scope.contains("admin"));  // Because user has admin role
}
