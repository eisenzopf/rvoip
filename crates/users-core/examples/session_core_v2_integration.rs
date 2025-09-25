//! Complete session-core-v2 integration example
//! 
//! This demonstrates the full integration flow between:
//! - users-core (authentication)
//! - auth-core (validation)
//! - session-core-v2 (SIP sessions)
//! - registrar-core (user registry)

use users_core::{init, UsersConfig, CreateUserRequest, UserClaims};
use jsonwebtoken::{decode, Algorithm, Validation, DecodingKey};
use anyhow::Result;
use std::collections::HashMap;
use std::sync::Arc;
use tokio::sync::RwLock;

// Simulated auth-core service
struct AuthCore {
    trusted_issuers: HashMap<String, TrustedIssuer>,
}

struct TrustedIssuer {
    issuer: String,
    public_key_pem: String,
    audiences: Vec<String>,
}

impl AuthCore {
    fn new() -> Self {
        Self {
            trusted_issuers: HashMap::new(),
        }
    }
    
    fn add_trusted_issuer(&mut self, issuer: TrustedIssuer) {
        self.trusted_issuers.insert(issuer.issuer.clone(), issuer);
    }
    
    async fn validate_token(&self, token: &str) -> Result<ValidatedToken> {
        // Decode header to find issuer
        let header = jsonwebtoken::decode_header(token)?;
        
        // For this example, we'll assume it's from users-core
        let issuer = "https://users.rvoip.local";
        let trusted = self.trusted_issuers.get(issuer)
            .ok_or_else(|| anyhow::anyhow!("Unknown issuer"))?;
        
        let decoding_key = DecodingKey::from_rsa_pem(trusted.public_key_pem.as_bytes())?;
        let mut validation = Validation::new(Algorithm::RS256);
        validation.set_issuer(&[&trusted.issuer]);
        validation.set_audience(&trusted.audiences);
        
        let token_data = decode::<UserClaims>(token, &decoding_key, &validation)?;
        
        Ok(ValidatedToken {
            user_id: token_data.claims.sub,
            username: token_data.claims.username,
            roles: token_data.claims.roles,
            scope: token_data.claims.scope,
        })
    }
}

#[derive(Debug)]
struct ValidatedToken {
    user_id: String,
    username: String,
    roles: Vec<String>,
    scope: String,
}

// Simulated registrar-core
struct RegistrarCore {
    registrations: Arc<RwLock<HashMap<String, Registration>>>,
}

struct Registration {
    user_id: String,
    username: String,
    contact: String,
    expires: u32,
}

impl RegistrarCore {
    fn new() -> Self {
        Self {
            registrations: Arc::new(RwLock::new(HashMap::new())),
        }
    }
    
    async fn register(&self, user_id: String, username: String, contact: String) -> Result<()> {
        let mut regs = self.registrations.write().await;
        regs.insert(user_id.clone(), Registration {
            user_id,
            username,
            contact,
            expires: 3600,
        });
        Ok(())
    }
}

// Simulated session-core-v2 adapter
struct SessionAdapter {
    auth_core: Arc<AuthCore>,
    registrar_core: Arc<RegistrarCore>,
}

impl SessionAdapter {
    async fn handle_register(&self, auth_header: Option<String>, contact: String) -> Result<String> {
        // Extract bearer token
        let auth_header = auth_header
            .ok_or_else(|| anyhow::anyhow!("Missing Authorization header"))?;
        
        let token = auth_header
            .strip_prefix("Bearer ")
            .ok_or_else(|| anyhow::anyhow!("Invalid Authorization format"))?;
        
        // Validate token via auth-core
        let validated = self.auth_core.validate_token(token).await?;
        
        // Check SIP registration permission
        if !validated.scope.contains("sip.register") {
            return Err(anyhow::anyhow!("Missing sip.register permission"));
        }
        
        // Register with registrar-core
        self.registrar_core
            .register(validated.user_id, validated.username, contact)
            .await?;
        
        Ok("200 OK".to_string())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    println!("🔗 Session-Core-V2 Integration Example\n");

    // Step 1: Initialize users-core
    println!("📦 Step 1: Initialize users-core...");
    let users_config = UsersConfig {
        database_url: "sqlite://integration_example.db?mode=rwc".to_string(),
        ..Default::default()
    };
    
    let users_core = init(users_config).await?;
    println!("✅ Users-core initialized");

    // Step 2: Initialize auth-core with users-core as trusted issuer
    println!("\n📦 Step 2: Configure auth-core...");
    let mut auth_core = AuthCore::new();
    
    // Get users-core public key
    let public_key = users_core.jwt_issuer().public_key_pem()?;
    
    // Add users-core as trusted issuer
    auth_core.add_trusted_issuer(TrustedIssuer {
        issuer: "https://users.rvoip.local".to_string(),
        public_key_pem: public_key,
        audiences: vec!["rvoip-api".to_string(), "rvoip-sip".to_string()],
    });
    
    println!("✅ Auth-core configured with users-core as trusted issuer");

    // Step 3: Initialize other components
    println!("\n📦 Step 3: Initialize registrar-core...");
    let registrar_core = RegistrarCore::new();
    println!("✅ Registrar-core initialized");

    // Step 4: Create session adapter
    println!("\n📦 Step 4: Create session-core-v2 adapter...");
    let session_adapter = SessionAdapter {
        auth_core: Arc::new(auth_core),
        registrar_core: Arc::new(registrar_core),
    };
    println!("✅ Session adapter created");

    // Step 5: Create test users
    println!("\n👥 Step 5: Create test users...");
    
    let alice = users_core.create_user(CreateUserRequest {
        username: "alice".to_string(),
        password: "SecureIntegration2024!".to_string(),
        email: Some("alice@example.com".to_string()),
        display_name: Some("Alice Smith".to_string()),
        roles: vec!["user".to_string()],
    }).await?;
    
    let bob = users_core.create_user(CreateUserRequest {
        username: "bob".to_string(),
        password: "SecureIntegration2024!".to_string(),
        email: Some("bob@example.com".to_string()),
        display_name: Some("Bob Jones".to_string()),
        roles: vec!["user".to_string()],
    }).await?;
    
    println!("✅ Created users: {} and {}", alice.username, bob.username);

    // Step 6: Simulate complete SIP REGISTER flow
    println!("\n📞 Step 6: Complete SIP REGISTER flow...");
    
    // Alice authenticates with users-core
    println!("\n[Alice's SIP Client]");
    println!("   1. Authenticate with users-core...");
    let alice_auth = users_core
        .authenticate_password("alice", "SecureIntegration2024!")
        .await?;
    println!("   ✓ Received JWT token");

    // Alice sends SIP REGISTER
    println!("   2. Send SIP REGISTER with Bearer token...");
    let auth_header = format!("Bearer {}", alice_auth.access_token);
    let contact = "sip:alice@192.168.1.100:5060".to_string();
    
    println!("\n[Session-Core-V2]");
    println!("   3. Receive REGISTER request");
    println!("   4. Extract bearer token from Authorization header");
    println!("   5. Send token to auth-core for validation");
    
    let response = session_adapter
        .handle_register(Some(auth_header), contact.clone())
        .await?;
    
    println!("   6. Token validated successfully");
    println!("   7. Create registration in registrar-core");
    println!("   8. Send response: {}", response);

    // Show the registration
    println!("\n[Registrar-Core]");
    let regs = session_adapter.registrar_core.registrations.read().await;
    if let Some(reg) = regs.get(&alice.id) {
        println!("   Registration created:");
        println!("   - User: {}", reg.username);
        println!("   - Contact: {}", reg.contact);
        println!("   - Expires: {} seconds", reg.expires);
    }

    // Step 7: Demonstrate presence subscription
    println!("\n📊 Step 7: Presence subscription flow...");
    
    // Bob subscribes to Alice's presence
    println!("\n[Bob's SIP Client]");
    let bob_auth = users_core
        .authenticate_password("bob", "SecureIntegration2024!")
        .await?;
    
    println!("   1. Send SUBSCRIBE for alice");
    println!("   2. Include Authorization: Bearer <bob's token>");
    
    println!("\n[Session-Core-V2]");
    println!("   3. Validate Bob's token via auth-core");
    println!("   4. Check if Bob is authorized to see Alice's presence");
    println!("   5. Create subscription in presence manager");
    println!("   6. Send initial NOTIFY with Alice's current presence");

    // Step 8: Show configuration summary
    println!("\n⚙️ Step 8: Configuration Summary");
    println!("\n[users-core]");
    println!("   - Database: SQLite");
    println!("   - JWT Algorithm: RS256");
    println!("   - Token TTL: 15 minutes");
    println!("   - Issuer: https://users.rvoip.local");
    
    println!("\n[auth-core]");
    println!("   - Trusted Issuers: users-core, (OAuth2 providers...)");
    println!("   - Token Cache: Enabled");
    println!("   - Validation: Local for JWT, remote for OAuth2");
    
    println!("\n[session-core-v2]");
    println!("   - Adapter: DialogAdapter, MediaAdapter, RegistrarAdapter");
    println!("   - State Machine: YAML-driven");
    println!("   - Events: Bi-directional mapping");
    
    println!("\n[registrar-core]");
    println!("   - Storage: In-memory (SQLite in production)");
    println!("   - Features: Multi-device, presence, PIDF");

    // Step 9: Error handling examples
    println!("\n❌ Step 9: Error handling examples...");
    
    // Invalid password
    println!("\n[Invalid Password]");
    match users_core.authenticate_password("alice", "WrongPassword").await {
        Ok(_) => println!("   ⚠️ Should have failed!"),
        Err(e) => println!("   ✓ Correctly rejected: {}", e),
    }
    
    // Missing authorization header
    println!("\n[Missing Authorization]");
    match session_adapter.handle_register(None, "sip:test@example.com".to_string()).await {
        Ok(_) => println!("   ⚠️ Should have failed!"),
        Err(e) => println!("   ✓ Correctly rejected: {}", e),
    }

    // Best practices summary
    println!("\n💡 Integration Best Practices:");
    println!("   1. Always validate tokens through auth-core");
    println!("   2. Check specific permissions (sip.register, presence, etc.)");
    println!("   3. Use connection pooling for database");
    println!("   4. Cache validated tokens for performance");
    println!("   5. Handle token refresh gracefully");
    println!("   6. Log authentication events for security");
    println!("   7. Implement rate limiting on authentication");
    println!("   8. Use TLS for all communications");

    // Clean up
    std::fs::remove_file("integration_example.db").ok();
    
    println!("\n✨ Session-core-v2 integration example completed!");
    Ok(())
}
