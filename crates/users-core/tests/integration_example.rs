//! Integration example showing how session-core-v2 would use users-core
//! This demonstrates the complete flow from user authentication to SIP registration

use users_core::{init, UsersConfig, CreateUserRequest};
use users_core::config::{PasswordConfig, TlsSettings};
use users_core::jwt::JwtConfig;
use tempfile::TempDir;

/// Example: How session-core-v2 would handle SIP REGISTER with users-core authentication
#[tokio::test]
async fn example_sip_register_flow() {
    // Setup users-core
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("users.db");
    let db_url = format!("sqlite://{}?mode=rwc", db_path.display());
    
    let config = UsersConfig {
        database_url: db_url,
        jwt: JwtConfig {
            issuer: "https://users.rvoip.local".to_string(),
            audience: vec!["rvoip-api".to_string(), "rvoip-sip".to_string()],
            access_ttl_seconds: 900,
            refresh_ttl_seconds: 2592000,
            algorithm: "RS256".to_string(),
            signing_key: None,
        },
        password: PasswordConfig {
            min_length: 12,  // Updated to match validation.rs
            require_uppercase: true,
            require_lowercase: true,
            require_numbers: true,
            require_special: false,
            argon2_memory_cost: 65536,
            argon2_time_cost: 3,
            argon2_parallelism: 4,
        },
        api_bind_address: "127.0.0.1:8081".to_string(),
        tls: TlsSettings::default(),
    };
    
    let auth_service = init(config).await.unwrap();
    
    // Create a SIP user
    let user = auth_service.create_user(CreateUserRequest {
        username: "alice".to_string(),
        password: "SecurePass2024!".to_string(),
        email: Some("alice@example.com".to_string()),
        display_name: Some("Alice Smith".to_string()),
        roles: vec!["user".to_string()],
    }).await.unwrap();
    
    println!("Created SIP user: {}", user.username);
    
    // Step 1: SIP client authenticates to get JWT
    let auth_result = auth_service
        .authenticate_password("alice", "SecurePass2024!")
        .await
        .unwrap();
    
    println!("Authentication successful!");
    println!("Access token (truncated): {}...", &auth_result.access_token[..20]);
    println!("Token expires in: {} seconds", auth_result.expires_in.as_secs());
    
    // Step 2: SIP client includes JWT in REGISTER request
    // In real implementation, this would be in the Authorization header
    let sip_register_auth_header = format!("Bearer {}", auth_result.access_token);
    
    // Step 3: session-core-v2 receives REGISTER and extracts token
    let bearer_token = extract_bearer_token(&sip_register_auth_header).unwrap();
    
    // Step 4: session-core-v2 sends token to auth-core for validation
    // auth-core would validate using users-core's public key
    // For this example, we'll demonstrate by decoding the JWT
    
    use jsonwebtoken::{decode, Algorithm, Validation, DecodingKey};
    let public_key = auth_service.jwt_issuer().public_key_pem().unwrap();
    let decoding_key = DecodingKey::from_rsa_pem(public_key.as_bytes()).unwrap();
    
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_issuer(&["https://users.rvoip.local"]);
    validation.set_audience(&["rvoip-sip"]);
    
    let token_data = decode::<users_core::UserClaims>(
        bearer_token,
        &decoding_key,
        &validation
    ).unwrap();
    
    println!("\nValidated token claims:");
    println!("  User ID: {}", token_data.claims.sub);
    println!("  Username: {}", token_data.claims.username);
    println!("  Roles: {:?}", token_data.claims.roles);
    println!("  Scope: {}", token_data.claims.scope);
    
    // Step 5: session-core-v2 creates SIP registration
    assert!(token_data.claims.scope.contains("sip.register"));
    assert!(token_data.claims.roles.contains(&"user".to_string()));
    
    println!("\n✓ User authorized for SIP registration!");
}

/// Example: API key authentication for automated systems
#[tokio::test]
async fn example_api_key_for_pbx_system() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("users.db");
    let db_url = format!("sqlite://{}?mode=rwc", db_path.display());
    
    let config = create_test_config(db_url);
    let auth_service = init(config).await.unwrap();
    
    // Create a service account for PBX system
    let pbx_user = auth_service.create_user(CreateUserRequest {
        username: "pbxsystem".to_string(),
        password: "NotUsedForApiKey123".to_string(),
        email: Some("pbx@example.com".to_string()),
        display_name: Some("PBX System Service".to_string()),
        roles: vec!["user".to_string()],
    }).await.unwrap();
    
    // Create an API key for the PBX
    let (api_key, raw_key) = auth_service.api_key_store()
        .create_api_key(users_core::api_keys::CreateApiKeyRequest {
            user_id: pbx_user.id.clone(),
            name: "PBX Integration Key".to_string(),
            permissions: vec![
                "read".to_string(),
                "write".to_string(),
            ],
            expires_at: None,  // No expiration
        })
        .await
        .unwrap();
    
    println!("Created API key for PBX system:");
    println!("  Key ID: {}", api_key.id);
    println!("  Key (store this securely): {}", raw_key);
    println!("  Permissions: {:?}", api_key.permissions);
    
    // PBX system uses API key to authenticate
    let auth_result = auth_service.authenticate_api_key(&raw_key).await.unwrap();
    
    println!("\nAPI key authentication successful!");
    println!("  Service account: {}", auth_result.user.username);
    println!("  Access token valid for: {} seconds", auth_result.expires_in.as_secs());
    
    // The PBX can now use this token for SIP operations
    assert!(auth_result.user.roles.contains(&"user".to_string()));
}

/// Example: Multi-device user with presence
#[tokio::test]
async fn example_multi_device_presence() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("users.db");
    let db_url = format!("sqlite://{}?mode=rwc", db_path.display());
    
    let config = create_test_config(db_url);
    let auth_service = init(config).await.unwrap();
    
    // Create a user who will register from multiple devices
    let user = auth_service.create_user(CreateUserRequest {
        username: "bob".to_string(),
        password: "SecurePass2024!".to_string(),
        email: Some("bob@company.com".to_string()),
        display_name: Some("Bob Johnson".to_string()),
        roles: vec!["user".to_string()],
    }).await.unwrap();
    
    // Device 1: Desktop softphone
    let desktop_auth = auth_service
        .authenticate_password("bob", "SecurePass2024!")
        .await
        .unwrap();
    
    // Device 2: Mobile app (same user, different token)
    let mobile_auth = auth_service
        .authenticate_password("bob", "SecurePass2024!")
        .await
        .unwrap();
    
    println!("User {} authenticated from multiple devices:", user.username);
    println!("  Desktop token: {}...", &desktop_auth.access_token[..20]);
    println!("  Mobile token: {}...", &mobile_auth.access_token[..20]);
    
    // Each device would REGISTER with its own token
    // session-core-v2 would:
    // 1. Validate each token
    // 2. Create separate registrations for each device
    // 3. Aggregate presence across all devices
    
    assert_ne!(desktop_auth.access_token, mobile_auth.access_token);
    assert_eq!(desktop_auth.user.id, mobile_auth.user.id);
    
    // Both tokens have user role
    let desktop_has_user_role = desktop_auth.user.roles.contains(&"user".to_string());
    let mobile_has_user_role = mobile_auth.user.roles.contains(&"user".to_string());
    
    assert!(desktop_has_user_role);
    assert!(mobile_has_user_role);
    
    println!("\n✓ Both devices authenticated successfully!");
}

/// Example: Token refresh workflow
#[tokio::test]
async fn example_token_refresh_workflow() {
    let temp_dir = TempDir::new().unwrap();
    let db_path = temp_dir.path().join("users.db");
    let db_url = format!("sqlite://{}?mode=rwc", db_path.display());
    
    let mut config = create_test_config(db_url);
    config.jwt.access_ttl_seconds = 5;  // Very short for testing
    
    let auth_service = init(config).await.unwrap();
    
    // Create and authenticate user
    auth_service.create_user(CreateUserRequest {
        username: "refresh_test".to_string(),
        password: "SecurePass2024".to_string(),
        email: None,
        display_name: None,
        roles: vec!["user".to_string()],
    }).await.unwrap();
    
    let initial_auth = auth_service
        .authenticate_password("refresh_test", "SecurePass2024")
        .await
        .unwrap();
    
    println!("Initial authentication:");
    println!("  Access token expires in: {} seconds", initial_auth.expires_in.as_secs());
    println!("  Refresh token: {}...", &initial_auth.refresh_token[..20]);
    
    // Simulate token expiration by waiting
    tokio::time::sleep(tokio::time::Duration::from_secs(2)).await;
    
    // Refresh the token
    let refreshed = auth_service
        .refresh_token(&initial_auth.refresh_token)
        .await
        .unwrap();
    
    println!("\nToken refreshed:");
    println!("  New access token: {}...", &refreshed.access_token[..20]);
    println!("  Same refresh token: {}", 
        refreshed.refresh_token == initial_auth.refresh_token
    );
    
    assert_ne!(refreshed.access_token, initial_auth.access_token);
    assert_eq!(refreshed.refresh_token, initial_auth.refresh_token);
}

// Helper functions
fn extract_bearer_token(auth_header: &str) -> Option<&str> {
    auth_header
        .strip_prefix("Bearer ")
        .map(|token| token.trim())
}

fn create_test_config(db_url: String) -> UsersConfig {
    UsersConfig {
        database_url: db_url,
        jwt: JwtConfig::default(),
        password: PasswordConfig {
            min_length: 12,  // Updated to match validation.rs
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
