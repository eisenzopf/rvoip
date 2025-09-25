//! SIP REGISTER flow with JWT authentication
//! 
//! This example demonstrates how session-core-v2 would handle:
//! - Extracting bearer tokens from SIP headers
//! - Validating tokens
//! - Creating SIP registrations

use users_core::{init, UsersConfig, CreateUserRequest, UserClaims};
use jsonwebtoken::{decode, Algorithm, Validation, DecodingKey};
use anyhow::Result;
use std::collections::HashMap;

// Simulated SIP REGISTER message
struct SipRegister {
    from: String,
    to: String,
    contact: String,
    authorization: Option<String>,
}

// Simulated SIP registration entry
struct SipRegistration {
    user_id: String,
    username: String,
    contact: String,
    expires: u32,
}

#[tokio::main]
async fn main() -> Result<()> {
    println!("🎯 SIP REGISTER Flow Example\n");

    // Initialize users-core
    let config = UsersConfig {
        database_url: "sqlite://sip_example.db?mode=rwc".to_string(),
        ..Default::default()
    };
    
    let auth_service = init(config).await?;

    // Create SIP users
    println!("📝 Creating SIP users...");
    
    let alice = auth_service.create_user(CreateUserRequest {
        username: "alice".to_string(),
        password: "SecurePass2024!".to_string(),
        email: Some("alice@example.com".to_string()),
        display_name: Some("Alice Smith".to_string()),
        roles: vec!["user".to_string()],
    }).await?;

    let bob = auth_service.create_user(CreateUserRequest {
        username: "bob".to_string(),
        password: "SecurePass2024!".to_string(),
        email: Some("bob@example.com".to_string()),
        display_name: Some("Bob Jones".to_string()),
        roles: vec!["user".to_string()],
    }).await?;

    println!("✅ Created users: {} and {}", alice.username, bob.username);

    // Simulate SIP client authentication
    println!("\n🔐 SIP clients authenticate to get JWT tokens...");
    
    let alice_auth = auth_service
        .authenticate_password("alice", "SecurePass2024!")
        .await?;
    
    println!("✅ Alice authenticated, got JWT token");

    // Simulate SIP REGISTER request
    println!("\n📞 Alice sends SIP REGISTER...");
    let register = SipRegister {
        from: "sip:alice@voip.example.com".to_string(),
        to: "sip:alice@voip.example.com".to_string(),
        contact: "sip:alice@192.168.1.100:5060".to_string(),
        authorization: Some(format!("Bearer {}", alice_auth.access_token)),
    };

    // This is what session-core-v2 would do:
    println!("\n🔍 Session-core-v2 processes REGISTER...");
    
    // 1. Extract bearer token
    let token = extract_bearer_token(&register.authorization)?;
    println!("   ✓ Extracted bearer token");

    // 2. Validate token (normally done by auth-core)
    let public_key = auth_service.jwt_issuer().public_key_pem()?;
    let claims = validate_token(token, &public_key)?;
    println!("   ✓ Token validated successfully");
    println!("     - User ID: {}", claims.sub);
    println!("     - Username: {}", claims.username);
    println!("     - Roles: {:?}", claims.roles);
    println!("     - Scope: {}", claims.scope);

    // 3. Check SIP registration permission
    if !claims.scope.contains("sip.register") {
        anyhow::bail!("User lacks sip.register permission");
    }
    println!("   ✓ User has sip.register permission");

    // 4. Create SIP registration
    let mut registrations = HashMap::new();
    let registration = SipRegistration {
        user_id: claims.sub.clone(),
        username: claims.username.clone(),
        contact: register.contact.clone(),
        expires: 3600,
    };
    
    registrations.insert(claims.sub.clone(), registration);
    println!("   ✓ Created SIP registration");
    println!("     - Contact: {}", register.contact);
    println!("     - Expires: 3600 seconds");

    // Handle expired token scenario
    println!("\n⏰ Simulating expired token scenario...");
    
    // Create a register with no auth header
    let register_no_auth = SipRegister {
        from: "sip:bob@voip.example.com".to_string(),
        to: "sip:bob@voip.example.com".to_string(),
        contact: "sip:bob@192.168.1.101:5060".to_string(),
        authorization: None,
    };

    match extract_bearer_token(&register_no_auth.authorization) {
        Ok(_) => println!("⚠️ Unexpected success"),
        Err(e) => println!("✅ Correctly rejected: {}", e),
    }

    // Handle multi-device registration
    println!("\n📱 Handling multi-device registration...");
    
    // Alice registers from mobile device
    let alice_mobile_auth = auth_service
        .authenticate_password("alice", "SecurePass2024!")
        .await?;
    
    let mobile_register = SipRegister {
        from: "sip:alice@voip.example.com".to_string(),
        to: "sip:alice@voip.example.com".to_string(),
        contact: "sip:alice@10.0.0.50:5060;transport=tcp".to_string(),
        authorization: Some(format!("Bearer {}", alice_mobile_auth.access_token)),
    };

    let mobile_token = extract_bearer_token(&mobile_register.authorization)?;
    let mobile_claims = validate_token(mobile_token, &public_key)?;
    
    println!("✅ Alice registered from second device");
    println!("   - Desktop: sip:alice@192.168.1.100:5060");
    println!("   - Mobile:  {}", mobile_register.contact);
    println!("   - Same user ID: {}", mobile_claims.sub == claims.sub);

    // Show how to handle SIP authentication challenges
    println!("\n🔒 Handling 401 Unauthorized flow...");
    println!("1. Client sends REGISTER without Authorization header");
    println!("2. Server responds: SIP/2.0 401 Unauthorized");
    println!("   WWW-Authenticate: Bearer realm=\"voip.example.com\"");
    println!("3. Client authenticates with users-core to get JWT");
    println!("4. Client resends REGISTER with Authorization: Bearer <jwt>");

    // Clean up
    std::fs::remove_file("sip_example.db").ok();
    
    println!("\n✨ SIP REGISTER example completed!");
    Ok(())
}

fn extract_bearer_token(auth_header: &Option<String>) -> Result<&str> {
    auth_header
        .as_ref()
        .ok_or_else(|| anyhow::anyhow!("Missing Authorization header"))?
        .strip_prefix("Bearer ")
        .ok_or_else(|| anyhow::anyhow!("Invalid Authorization header format"))
}

fn validate_token(token: &str, public_key_pem: &str) -> Result<UserClaims> {
    let decoding_key = DecodingKey::from_rsa_pem(public_key_pem.as_bytes())?;
    
    let mut validation = Validation::new(Algorithm::RS256);
    validation.set_issuer(&["https://users.rvoip.local"]);
    validation.set_audience(&["rvoip-api", "rvoip-sip"]);
    
    let token_data = decode::<UserClaims>(token, &decoding_key, &validation)?;
    
    Ok(token_data.claims)
}
