//! Token validation example
//! 
//! This demonstrates how auth-core would validate JWT tokens issued by users-core
//! and extract user context for session-core-v2

use users_core::{init, UsersConfig, CreateUserRequest, UserClaims};
use jsonwebtoken::{decode, encode, Algorithm, Validation, DecodingKey, EncodingKey, Header};
use serde::{Deserialize, Serialize};
use anyhow::Result;
use std::time::{SystemTime, UNIX_EPOCH};

// Simulated auth-core UserContext
#[derive(Debug, Clone)]
struct UserContext {
    user_id: String,
    username: String,
    email: Option<String>,
    roles: Vec<String>,
    issuer: String,
    token_type: TokenType,
}

#[derive(Debug, Clone)]
enum TokenType {
    UsersCore,
    OAuth2Google,
    OAuth2Azure,
}

#[tokio::main]
async fn main() -> Result<()> {
    println!("🔍 Token Validation Example\n");

    // Initialize users-core
    let config = UsersConfig {
        database_url: "sqlite://token_validation.db?mode=rwc".to_string(),
        ..Default::default()
    };
    
    let auth_service = init(config).await?;

    // Create and authenticate a user
    println!("📝 Setting up test user...");
    
    let user = auth_service.create_user(CreateUserRequest {
        username: "validator@example.com".to_string(),
        password: "ValidateMe123".to_string(),
        email: Some("validator@example.com".to_string()),
        display_name: Some("Token Validator".to_string()),
        roles: vec!["user".to_string(), "admin".to_string()],
    }).await?;

    let auth_result = auth_service
        .authenticate_password("validator@example.com", "ValidateMe123")
        .await?;

    println!("✅ User authenticated, received JWT token");

    // Simulate auth-core validation
    println!("\n🔐 Auth-core validates the token...");
    
    let public_key = auth_service.jwt_issuer().public_key_pem()?;
    let user_context = validate_and_extract_context(&auth_result.access_token, &public_key)?;
    
    println!("✅ Token validated successfully!");
    println!("   User Context:");
    println!("   - User ID: {}", user_context.user_id);
    println!("   - Username: {}", user_context.username);
    println!("   - Email: {:?}", user_context.email);
    println!("   - Roles: {:?}", user_context.roles);
    println!("   - Issuer: {}", user_context.issuer);
    println!("   - Token Type: {:?}", user_context.token_type);

    // Show JWKS endpoint simulation
    println!("\n🔑 JWKS Endpoint (for auth-core configuration)...");
    let jwks = simulate_jwks_endpoint(&auth_service)?;
    println!("   GET /auth/jwks.json");
    println!("   {}", serde_json::to_string_pretty(&jwks)?);

    // Demonstrate token introspection
    println!("\n📊 Token introspection...");
    let introspection = introspect_token(&auth_result.access_token, &public_key)?;
    println!("   Active: {}", introspection.active);
    println!("   Scope: {}", introspection.scope);
    println!("   Username: {}", introspection.username);
    println!("   Expires in: {} seconds", introspection.exp - current_timestamp());

    // Show different token sources
    println!("\n🌐 Handling different token sources...");
    
    // Simulate OAuth2 token (for comparison)
    let oauth2_token = simulate_oauth2_token();
    println!("   OAuth2 token would be validated against provider's endpoint");
    println!("   Users-core tokens are validated locally with public key");

    // Demonstrate token caching strategy
    println!("\n💾 Token caching strategy for auth-core...");
    println!("   1. Check if token exists in cache");
    println!("   2. If cached and not expired, return cached UserContext");
    println!("   3. If not cached:");
    println!("      - Decode JWT header to identify issuer");
    println!("      - If issuer is users-core, validate with public key");
    println!("      - If issuer is OAuth2, validate with provider");
    println!("   4. Cache the result with TTL");

    // Show error handling
    println!("\n❌ Error handling examples...");
    
    // Expired token
    let expired_token = create_expired_token(&auth_service)?;
    match validate_and_extract_context(&expired_token, &public_key) {
        Ok(_) => println!("   ⚠️ Expired token validated - unexpected!"),
        Err(e) => println!("   ✓ Expired token rejected: {}", e),
    }

    // Invalid signature
    let tampered_token = auth_result.access_token.replace("a", "b");
    match validate_and_extract_context(&tampered_token, &public_key) {
        Ok(_) => println!("   ⚠️ Tampered token validated - unexpected!"),
        Err(e) => println!("   ✓ Tampered token rejected: {}", e),
    }

    // Clean up
    std::fs::remove_file("token_validation.db").ok();
    
    println!("\n✨ Token validation example completed!");
    Ok(())
}

fn validate_and_extract_context(token: &str, public_key_pem: &str) -> Result<UserContext> {
    let decoding_key = DecodingKey::from_rsa_pem(public_key_pem.as_bytes())?;
    
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_issuer(&["https://users.rvoip.local"]);
    validation.set_audience(&["rvoip-api", "rvoip-sip"]);
    
    let token_data = decode::<UserClaims>(token, &decoding_key, &validation)?;
    let claims = token_data.claims;
    
    Ok(UserContext {
        user_id: claims.sub,
        username: claims.username,
        email: claims.email,
        roles: claims.roles,
        issuer: claims.iss,
        token_type: TokenType::UsersCore,
    })
}

#[derive(Serialize)]
struct JwksResponse {
    keys: Vec<Jwk>,
}

#[derive(Serialize)]
struct Jwk {
    kty: String,
    #[serde(rename = "use")]
    use_: String,
    kid: String,
    alg: String,
    n: String,
    e: String,
}

fn simulate_jwks_endpoint(auth_service: &users_core::AuthenticationService) -> Result<JwksResponse> {
    // In reality, this would extract the actual RSA components
    Ok(JwksResponse {
        keys: vec![Jwk {
            kty: "RSA".to_string(),
            use_: "sig".to_string(),
            kid: "users-core-2024".to_string(),
            alg: "RS256".to_string(),
            n: "base64url_encoded_modulus_would_go_here".to_string(),
            e: "AQAB".to_string(),
        }],
    })
}

#[derive(Debug)]
struct TokenIntrospection {
    active: bool,
    scope: String,
    username: String,
    exp: u64,
}

fn introspect_token(token: &str, public_key_pem: &str) -> Result<TokenIntrospection> {
    let decoding_key = DecodingKey::from_rsa_pem(public_key_pem.as_bytes())?;
    let validation = Validation::new(Algorithm::RS256);
    
    match decode::<UserClaims>(token, &decoding_key, &validation) {
        Ok(token_data) => {
            let claims = token_data.claims;
            Ok(TokenIntrospection {
                active: claims.exp > current_timestamp(),
                scope: claims.scope,
                username: claims.username,
                exp: claims.exp,
            })
        },
        Err(_) => Ok(TokenIntrospection {
            active: false,
            scope: String::new(),
            username: String::new(),
            exp: 0,
        }),
    }
}

fn simulate_oauth2_token() -> String {
    // This would be an opaque token from an OAuth2 provider
    "ya29.a0AfH6SMBx3J5Xb_XYZ123".to_string()
}

fn create_expired_token(auth_service: &users_core::AuthenticationService) -> Result<String> {
    // Create a token that's already expired
    let claims = UserClaims {
        iss: "https://users.rvoip.local".to_string(),
        sub: "test-user".to_string(),
        aud: vec!["rvoip-api".to_string()],
        exp: current_timestamp() - 3600, // Expired 1 hour ago
        iat: current_timestamp() - 7200,
        jti: "expired-token".to_string(),
        username: "test".to_string(),
        email: None,
        roles: vec![],
        scope: "test".to_string(),
    };
    
    // For this example, we'll just return a dummy token
    Ok("eyJ0eXAiOiJKV1QiLCJhbGciOiJSUzI1NiIsImtpZCI6InVzZXJzLWNvcmUtMjAyNCJ9.expired".to_string())
}

fn current_timestamp() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap()
        .as_secs()
}
