//! API Key usage for services
//!
//! This demonstrates how services like PBX systems, monitoring tools,
//! or automation scripts would use API keys instead of passwords

use anyhow::Result;
use chrono::{Duration, Utc};
use users_core::api_keys::CreateApiKeyRequest;
use users_core::{init, CreateUserRequest, UsersConfig};

// Simulated service that needs API access
struct PbxService {
    api_key: String,
    base_url: String,
}

impl PbxService {
    async fn register_extensions(&self) -> Result<()> {
        println!("   📞 Registering PBX extensions using API key...");
        // In reality, this would make HTTP requests with X-API-Key header
        println!("   ✓ Registered 50 extensions");
        Ok(())
    }

    async fn monitor_calls(&self) -> Result<()> {
        println!("   📊 Monitoring active calls...");
        println!("   ✓ 5 active calls detected");
        Ok(())
    }
}

#[tokio::main]
async fn main() -> Result<()> {
    println!("🔑 API Key Service Example\n");

    // Initialize users-core
    let config = UsersConfig {
        database_url: "sqlite://api_key_example.db?mode=rwc".to_string(),
        ..Default::default()
    };

    let auth_service = init(config).await?;

    // Create service accounts
    println!("📝 Creating service accounts...");

    // PBX Service Account
    let pbx_account = auth_service
        .create_user(CreateUserRequest {
            username: "pbx-service".to_string(),
            password: "NotUsedWithApiKeys2024!".to_string(), // Required but not used for API key auth
            email: Some("pbx@services.example.com".to_string()),
            display_name: Some("PBX Service Account".to_string()),
            roles: vec!["user".to_string()],
        })
        .await?;

    // Monitoring Service Account
    let monitor_account = auth_service
        .create_user(CreateUserRequest {
            username: "monitoring-service".to_string(),
            password: "AlsoNotUsed2024!".to_string(),
            email: Some("monitor@services.example.com".to_string()),
            display_name: Some("Monitoring Service".to_string()),
            roles: vec!["user".to_string()],
        })
        .await?;

    // Automation Bot Account
    let bot_account = auth_service
        .create_user(CreateUserRequest {
            username: "automation-bot".to_string(),
            password: "BotPassword2024!".to_string(),
            email: Some("bot@services.example.com".to_string()),
            display_name: Some("Automation Bot".to_string()),
            roles: vec!["user".to_string()],
        })
        .await?;

    println!("✅ Created service accounts");

    // Create API keys with different permissions
    println!("\n🔐 Creating API keys with specific permissions...");

    let api_key_store = auth_service.api_key_store();

    // PBX API Key - full access
    let (pbx_key, pbx_raw) = api_key_store
        .create_api_key(CreateApiKeyRequest {
            user_id: pbx_account.id.clone(),
            name: "PBX Master Key".to_string(),
            permissions: vec!["read".to_string(), "write".to_string(), "admin".to_string()],
            expires_at: None, // No expiration for production service
        })
        .await?;

    println!("✅ PBX API Key created:");
    println!("   Name: {}", pbx_key.name);
    println!("   Key: {}", pbx_raw);
    println!("   Permissions: {:?}", pbx_key.permissions);

    // Monitoring API Key - read only, expires
    let (monitor_key, monitor_raw) = api_key_store
        .create_api_key(CreateApiKeyRequest {
            user_id: monitor_account.id.clone(),
            name: "Monitoring Read-Only Key".to_string(),
            permissions: vec!["read".to_string()],
            expires_at: Some(Utc::now() + Duration::days(90)), // Expires in 90 days
        })
        .await?;

    println!("\n✅ Monitoring API Key created:");
    println!("   Name: {}", monitor_key.name);
    println!("   Key: {}", monitor_raw);
    println!("   Permissions: {:?}", monitor_key.permissions);
    println!("   Expires: {:?}", monitor_key.expires_at);

    // Bot API Key - limited scope
    let (bot_key, bot_raw) = api_key_store
        .create_api_key(CreateApiKeyRequest {
            user_id: bot_account.id.clone(),
            name: "Bot Automation Key".to_string(),
            permissions: vec!["write".to_string()],
            expires_at: Some(Utc::now() + Duration::days(30)), // Short expiration
        })
        .await?;

    println!("\n✅ Bot API Key created:");
    println!("   Name: {}", bot_key.name);
    println!("   Key: {}", bot_raw);
    println!("   Permissions: {:?}", bot_key.permissions);
    println!("   Expires: {:?}", bot_key.expires_at);

    // Demonstrate API key authentication
    println!("\n🚀 Using API keys for authentication...");

    // PBX Service authenticates
    let pbx_auth = auth_service.authenticate_api_key(&pbx_raw).await?;
    println!("\n✅ PBX authenticated with API key");
    println!("   Service account: {}", pbx_auth.user.username);
    println!("   Roles: {:?}", pbx_auth.user.roles);
    println!(
        "   Token expires in: {} seconds",
        pbx_auth.expires_in.as_secs()
    );

    // Use the PBX service
    let pbx_service = PbxService {
        api_key: pbx_raw.clone(),
        base_url: "https://api.voip.example.com".to_string(),
    };

    pbx_service.register_extensions().await?;
    pbx_service.monitor_calls().await?;

    // Show key rotation workflow
    println!("\n🔄 API Key rotation example...");

    // Create new key before revoking old one
    let (new_pbx_key, new_pbx_raw) = api_key_store
        .create_api_key(CreateApiKeyRequest {
            user_id: pbx_account.id.clone(),
            name: "PBX Master Key v2".to_string(),
            permissions: pbx_key.permissions.clone(),
            expires_at: None,
        })
        .await?;

    println!("✅ New API key created: {}", new_pbx_raw);

    // Revoke old key
    api_key_store.revoke_api_key(&pbx_key.id).await?;
    println!("✅ Old API key revoked");

    // List all keys for account
    println!("\n📋 Listing API keys for PBX account...");
    let pbx_keys = api_key_store.list_api_keys(&pbx_account.id).await?;

    for key in pbx_keys {
        println!("   - {} (created: {})", key.name, key.created_at);
        if let Some(last_used) = key.last_used {
            println!("     Last used: {}", last_used);
        }
    }

    // Show how services would use API keys in HTTP headers
    println!("\n📡 How services use API keys in practice:");
    println!("\n   REST API Request:");
    println!("   ```");
    println!("   POST /api/v1/calls");
    println!("   X-API-Key: {}", &new_pbx_raw[..20]);
    println!("   Content-Type: application/json");
    println!("   ```");

    println!("\n   SIP with API Key (custom header):");
    println!("   ```");
    println!("   REGISTER sip:pbx@voip.example.com SIP/2.0");
    println!("   X-API-Key: {}", &new_pbx_raw[..20]);
    println!("   ```");

    // Security best practices
    println!("\n🛡️ API Key Security Best Practices:");
    println!("   1. Never commit API keys to source control");
    println!("   2. Use environment variables or secure vaults");
    println!("   3. Rotate keys regularly");
    println!("   4. Use minimal required permissions");
    println!("   5. Set expiration for non-critical services");
    println!("   6. Monitor key usage");
    println!("   7. Revoke unused keys");

    // Clean up
    std::fs::remove_file("api_key_example.db").ok();

    println!("\n✨ API key service example completed!");
    Ok(())
}
